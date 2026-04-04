//! Core agent loop — prompt → LLM → tool execution → repeat.
//!
//! The [`Agent`] struct owns the conversation state, tool executor,
//! and LLM client. Each call to [`Agent::turn`] runs one complete
//! prompt-response-tool cycle. Messages are appended to the `turn_messages`
//! SQLite table for crash recovery.

use crate::mode::Mode;
use crate::routing::ModelRouter;
use crate::session::{SessionStatus, SessionStore, ToolCallEntry};
use crate::system_prompt::build_system_prompt;
use crate::thinking::ThinkingFilter;
use anvil_config::Settings;
use anvil_llm::{
    ChatMessage, ChatRequest, LlmClient, StreamEvent, TokenUsage, ToolCallAccumulator,
};
use anvil_mcp::McpManager;
use anvil_tools::{all_tool_definitions, PermissionDecision, ToolExecutor};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Events emitted by the agent loop for the UI to consume.
#[derive(Debug)]
pub enum AgentEvent {
    /// Thinking content from `<think>` blocks (only emitted when thinking is visible).
    ThinkingDelta(String),
    /// Assistant is producing text content.
    ContentDelta(String),
    /// Assistant wants to call a tool — awaiting permission.
    ToolCallPending {
        id: String,
        name: String,
        arguments: String,
    },
    /// Tool execution completed.
    ToolResult {
        name: String,
        result: anvil_tools::ToolOutput,
    },
    /// Updated token usage.
    Usage(TokenUsage),
    /// Agent turn completed (assistant finished responding).
    TurnComplete,
    /// LLM request is being retried.
    Retry {
        attempt: usize,
        max: usize,
        delay_secs: f64,
    },
    /// Repeated tool call loop detected.
    LoopDetected { tool_name: String, count: usize },
    /// Context window usage warning.
    ContextWarning {
        estimated_tokens: usize,
        limit: usize,
    },
    /// Context was auto-compacted because it exceeded the threshold.
    AutoCompacted {
        before_tokens: usize,
        after_tokens: usize,
        messages_removed: usize,
    },
    /// Real-time output from a running tool (e.g. shell command stdout).
    ToolOutputDelta { name: String, delta: String },
    /// Turn was cancelled (e.g. by Ctrl+C). Partial content may have been emitted.
    Cancelled,
    /// Error occurred.
    Error(String),
}

/// Result of context compaction.
#[derive(Debug)]
pub struct CompactionResult {
    /// Estimated token count before compaction.
    pub before_tokens: usize,
    /// Estimated token count after compaction.
    pub after_tokens: usize,
    /// Number of messages removed and replaced with summary.
    pub messages_removed: usize,
}

pub struct Agent {
    client: LlmClient,
    executor: ToolExecutor,
    store: SessionStore,
    settings: Settings,
    _workspace: PathBuf,
    session_id: String,
    messages: Vec<ChatMessage>,
    context_limit: usize,
    loop_detection_limit: usize,
    /// Auto-compact when context usage exceeds this percentage (0 = disabled).
    auto_compact_threshold: u8,
    tool_call_hashes: Vec<u64>,
    active_skills: Vec<crate::skills::Skill>,
    thinking_filter: ThinkingFilter,
    router: ModelRouter,
    /// MCP server manager — shared via Arc so it can be accessed from commands.
    mcp: Arc<McpManager>,
    /// Active character persona (fun mode).
    active_persona: Option<crate::persona::Persona>,
    /// Operating mode — controls tool availability and response style.
    mode: Mode,
}

impl Agent {
    pub fn new(
        settings: Settings,
        workspace: PathBuf,
        store: SessionStore,
        mcp: Arc<McpManager>,
    ) -> Result<Self> {
        let client = LlmClient::new(settings.provider.clone())?;
        let executor = ToolExecutor::new(
            workspace.clone(),
            settings.tools.shell_timeout_secs,
            settings.tools.output_limit,
        );

        let session = store.create_session()?;
        let system_prompt = build_system_prompt(
            &workspace,
            settings.agent.system_prompt_override.as_deref(),
            &settings.provider.model,
            &[],
        );
        let messages = vec![ChatMessage::system(system_prompt)];

        let context_limit = settings.agent.context_window;
        let loop_detection_limit = settings.agent.loop_detection_limit as usize;
        let auto_compact_threshold = settings.agent.auto_compact_threshold;

        Ok(Self {
            client,
            executor,
            store,
            settings,
            _workspace: workspace,
            session_id: session.id,
            messages,
            context_limit,
            loop_detection_limit,
            auto_compact_threshold,
            tool_call_hashes: Vec::new(),
            active_skills: Vec::new(),
            thinking_filter: ThinkingFilter::new(),
            router: ModelRouter::new(),
            mcp,
            active_persona: None,
            mode: Mode::default(),
        })
    }

    /// Resume an existing session with previously stored messages.
    pub fn resume(
        settings: Settings,
        workspace: PathBuf,
        store: SessionStore,
        session_id: &str,
        stored_messages: Vec<crate::session::StoredMessage>,
        mcp: Arc<McpManager>,
    ) -> Result<Self> {
        let client = LlmClient::new(settings.provider.clone())?;
        let executor = ToolExecutor::new(
            workspace.clone(),
            settings.tools.shell_timeout_secs,
            settings.tools.output_limit,
        );

        // Regenerate system prompt (picks up updated context files)
        let system_prompt = build_system_prompt(
            &workspace,
            settings.agent.system_prompt_override.as_deref(),
            &settings.provider.model,
            &[],
        );
        let mut messages = vec![ChatMessage::system(system_prompt)];

        // Try loading exact ChatMessage JSON from turn_messages (v2.1+).
        // Falls back to decomposed StoredMessage reconstruction for pre-v2.1 sessions.
        let turn_msgs = store.load_turn_messages(session_id).unwrap_or_default();
        if !turn_msgs.is_empty() {
            // Skip system messages from the stored turn — we regenerated ours
            for msg in turn_msgs {
                if msg.role != anvil_llm::Role::System {
                    messages.push(msg);
                }
            }
        } else {
            // Fallback: reconstruct from decomposed stored messages
            for msg in &stored_messages {
                match msg.role.as_str() {
                    "system" => {} // skip — we regenerated it
                    "user" => {
                        if let Some(content) = &msg.content {
                            messages.push(ChatMessage::user(content));
                        }
                    }
                    "assistant" => {
                        let content = msg.content.as_deref().unwrap_or("");
                        let mut chat_msg = ChatMessage::assistant(content);
                        if let Some(tc_json) = &msg.tool_calls_json {
                            if let Ok(tool_calls) = serde_json::from_str(tc_json) {
                                chat_msg.tool_calls = Some(tool_calls);
                            }
                        }
                        messages.push(chat_msg);
                    }
                    "tool" => {
                        if let (Some(content), Some(tc_id)) = (&msg.content, &msg.tool_call_id) {
                            messages.push(ChatMessage::tool_result(tc_id, content));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Mark session as active
        store.update_session_status(session_id, &SessionStatus::Active)?;

        // Load persisted agent state (mode, persona, skills) if available
        let snapshot = store.load_snapshot(session_id)?;

        let context_limit = settings.agent.context_window;
        let loop_detection_limit = settings.agent.loop_detection_limit as usize;
        let auto_compact_threshold = settings.agent.auto_compact_threshold;

        let mut agent = Self {
            client,
            executor,
            store,
            settings,
            _workspace: workspace,
            session_id: session_id.to_string(),
            messages,
            context_limit,
            loop_detection_limit,
            auto_compact_threshold,
            tool_call_hashes: Vec::new(),
            active_skills: Vec::new(),
            thinking_filter: ThinkingFilter::new(),
            router: ModelRouter::new(),
            mcp,
            active_persona: None,
            mode: Mode::default(),
        };

        // Restore agent state from snapshot
        if let Some(snap) = snapshot {
            // Restore mode
            match snap.mode.as_str() {
                "creative" => agent.set_mode(Mode::Creative),
                _ => agent.set_mode(Mode::Coding),
            }

            // Restore persona
            if let Some(ref key) = snap.persona {
                if let Some(persona) = crate::persona::find_persona(key) {
                    agent.set_persona(Some(persona));
                }
            }

            // Restore skills
            let loader = crate::skills::SkillLoader::new(agent.workspace());
            let all_skills = loader.scan();
            for key in &snap.active_skills {
                if let Some(skill) = all_skills.iter().find(|s| s.key == *key) {
                    agent.activate_skill(skill.clone());
                }
            }
        }

        Ok(agent)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn model(&self) -> &str {
        self.client.model()
    }

    pub fn workspace(&self) -> &std::path::Path {
        &self._workspace
    }

    /// Toggle whether `<think>` blocks are shown in output.
    pub fn set_show_thinking(&mut self, show: bool) {
        self.thinking_filter.set_show_thinking(show);
    }

    /// Whether `<think>` blocks are currently shown.
    pub fn show_thinking(&self) -> bool {
        self.thinking_filter.show_thinking()
    }

    pub fn base_url(&self) -> &str {
        self.client.base_url()
    }

    pub fn set_model(&mut self, model: String) {
        self.client.set_model(model);
    }

    /// Get the current backend kind.
    pub fn backend(&self) -> &anvil_config::BackendKind {
        self.client.backend()
    }

    /// Switch backend type and URL.
    ///
    /// # Why this exists
    /// Users may run multiple backends simultaneously (e.g. Ollama for quick tasks,
    /// llama-server for GLM-4.7-Flash). The `/backend` command calls this to switch
    /// without restarting Anvil.
    pub fn set_backend(&mut self, backend: anvil_config::BackendKind, base_url: String) {
        self.client.set_backend(backend);
        self.client.set_base_url(base_url);
    }

    /// Apply sampling parameters from a model profile.
    ///
    /// # What happens
    /// Extracts the `SamplingConfig` from the profile and passes it to the LLM client.
    /// Every subsequent API request will include these sampling params (temperature,
    /// top_p, min_p, repeat_penalty, top_k).
    pub fn apply_model_profile(&mut self, profile: &anvil_config::ModelProfile) {
        self.client.set_sampling(profile.sampling.clone());
    }

    /// Access the model router for adding/removing routes.
    pub fn router_mut(&mut self) -> &mut ModelRouter {
        &mut self.router
    }

    /// Access the model router (read-only).
    pub fn router(&self) -> &ModelRouter {
        &self.router
    }

    /// Clear sampling parameter overrides (revert to backend defaults).
    pub fn clear_sampling(&mut self) {
        self.client.clear_sampling();
    }

    pub fn usage(&self) -> &TokenUsage {
        self.client.usage()
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn pause_session(&self) -> Result<()> {
        self.store
            .update_session_status(&self.session_id, &SessionStatus::Paused)
    }

    pub fn end_session(&self) -> Result<()> {
        self.store
            .update_session_status(&self.session_id, &SessionStatus::Completed)
    }

    /// Compact the conversation context by summarizing old messages.
    ///
    /// # How it works
    /// 1. Preserves the system prompt and the most recent `keep_recent` messages
    /// 2. Sends the middle messages to the LLM with a compaction prompt
    /// 3. Replaces them with a single summary message
    /// 4. Returns before/after token estimates
    ///
    /// # Why not just truncate?
    /// Summarization preserves key decisions and context that simple truncation
    /// would lose. The LLM produces a condensed version of the conversation.
    pub async fn compact(
        &mut self,
        keep_recent: usize,
        event_tx: &mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    ) -> Result<CompactionResult> {
        let before_tokens = self.estimate_context_tokens();

        let (compact_start, compact_end) = match compaction_range(self.messages.len(), keep_recent)
        {
            Some(range) => range,
            None => {
                return Ok(CompactionResult {
                    before_tokens,
                    after_tokens: before_tokens,
                    messages_removed: 0,
                })
            }
        };

        // Build a summary of the messages to compact
        let messages_to_compact = &self.messages[compact_start..compact_end];
        let mut summary_input = String::new();
        for msg in messages_to_compact {
            let role = match msg.role {
                anvil_llm::Role::User => "User",
                anvil_llm::Role::Assistant => "Assistant",
                anvil_llm::Role::Tool => "Tool",
                anvil_llm::Role::System => "System",
            };
            let content = msg.content.as_deref().unwrap_or("");
            // Truncate long tool results in the summary input
            let truncated = if content.len() > 500 {
                format!("{}... [truncated]", &content[..500])
            } else {
                content.to_string()
            };
            summary_input.push_str(&format!("{role}: {truncated}\n\n"));
        }

        let compaction_prompt = format!(
            "Summarize the following conversation history in a concise paragraph. \
             Preserve key decisions, file paths, tool results, and any important context. \
             Do not include greetings or filler.\n\n{summary_input}"
        );

        // Send compaction request to LLM
        let mut request = ChatRequest {
            model: String::new(),
            messages: vec![
                ChatMessage::system(
                    "You are a conversation summarizer. Output only the summary, nothing else."
                        .to_string(),
                ),
                ChatMessage::user(&compaction_prompt),
            ],
            tools: None,
            tool_choice: None,
            temperature: Some(0.3),
            top_p: None,
            min_p: None,
            repeat_penalty: None,
            top_k: None,
            stream: true,
        };

        let mut rx = self
            .client
            .chat_stream(&mut request, cancel.clone(), |_, _, _| {})
            .await?;

        let mut summary = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::ContentDelta(delta) => {
                    summary.push_str(&delta);
                }
                StreamEvent::Usage(u) => {
                    self.client
                        .record_stream_usage(u.prompt_tokens, u.completion_tokens);
                }
                StreamEvent::Done => break,
                _ => {}
            }
        }

        if cancel.is_cancelled() || summary.is_empty() {
            return Ok(CompactionResult {
                before_tokens,
                after_tokens: before_tokens,
                messages_removed: 0,
            });
        }

        // Replace compacted messages with summary
        let messages_removed = compact_end - compact_start;
        let summary_msg = ChatMessage::user(format!(
            "[Context summary of {messages_removed} previous messages]\n{summary}"
        ));

        let mut new_messages = Vec::new();
        new_messages.push(self.messages[0].clone()); // system prompt
        new_messages.push(summary_msg);
        new_messages.extend_from_slice(&self.messages[compact_end..]);
        self.messages = new_messages;

        let after_tokens = self.estimate_context_tokens();

        let _ = event_tx
            .send(AgentEvent::Usage(self.client.usage().clone()))
            .await;

        Ok(CompactionResult {
            before_tokens,
            after_tokens,
            messages_removed,
        })
    }

    /// Activate a skill — injects its content into the system prompt and
    /// enables its declared environment variables for shell passthrough.
    ///
    /// # What happens
    /// 1. Skill content is appended to the system prompt as a `## Skill: <name>` section
    /// 2. Skill's `required_env` vars are added to the shell tool's passthrough list
    /// 3. Duplicate activations are ignored (idempotent)
    pub fn activate_skill(&mut self, skill: crate::skills::Skill) {
        if !self.active_skills.iter().any(|s| s.key == skill.key) {
            self.active_skills.push(skill);
            self.rebuild_system_prompt();
            self.sync_skill_env();
        }
    }

    /// Deactivate all skills — removes their content from the system prompt
    /// and clears all skill-declared environment variables.
    pub fn clear_skills(&mut self) {
        self.active_skills.clear();
        self.rebuild_system_prompt();
        self.sync_skill_env();
    }

    /// Synchronize the shell tool's extra env vars with active skills' declarations.
    ///
    /// # How it works
    /// Collects all `required_env` from active skills, deduplicates, and passes
    /// to the executor. Called whenever skills are activated or deactivated.
    fn sync_skill_env(&mut self) {
        let mut env_vars: Vec<String> = self
            .active_skills
            .iter()
            .flat_map(|s| s.required_env.iter().cloned())
            .collect();
        env_vars.sort();
        env_vars.dedup();
        self.executor.set_extra_env(env_vars);
    }

    pub fn has_active_skill(&self, key: &str) -> bool {
        self.active_skills.iter().any(|s| s.key == key)
    }

    /// Get the list of extra env vars currently passed through to shell commands.
    pub fn extra_env(&self) -> &[String] {
        self.executor.extra_env()
    }

    /// Set the write ledger on the tool executor.
    /// Called by the binary crate in daemon/watch mode to prevent
    /// the file watcher from triggering on the agent's own writes.
    pub fn set_write_ledger(&mut self, ledger: anvil_tools::WriteLedger) {
        self.executor.set_write_ledger(ledger);
    }

    /// Keys of currently active skills (for snapshot persistence).
    pub fn active_skill_keys(&self) -> Vec<String> {
        self.active_skills.iter().map(|s| s.key.clone()).collect()
    }

    /// Append the last message in the history to the turn log.
    ///
    /// Called after every `self.messages.push()` so crash recovery
    /// can reconstruct the exact conversation state.
    fn record_last_message(&self) {
        let seq = self.messages.len().saturating_sub(1);
        if let Some(msg) = self.messages.last() {
            if let Err(e) = self.store.append_turn_message(&self.session_id, seq, msg) {
                tracing::warn!("failed to append turn message: {e}");
            }
        }
    }

    /// Persist agent state metadata to SQLite.
    ///
    /// Called at the end of every turn so `anvil --continue` restores
    /// the full agent state. Messages are already saved individually
    /// during the turn — this captures mode, persona, skills, and profile.
    pub fn persist_snapshot(&self) -> Result<()> {
        let snapshot = crate::session::SessionSnapshot {
            active_skills: self.active_skill_keys(),
            mode: self.mode.to_string(),
            persona: self.active_persona.as_ref().map(|p| p.key.clone()),
            model_profile: None, // set by binary crate when profile is applied
        };
        self.store.save_snapshot(&self.session_id, &snapshot)
    }

    fn rebuild_system_prompt(&mut self) {
        let mut prompt = build_system_prompt(
            &self._workspace,
            self.settings.agent.system_prompt_override.as_deref(),
            self.client.model(),
            &self.active_skills,
        );

        // Prepend persona instructions if active
        if let Some(persona) = &self.active_persona {
            prompt = format!("{}\n\n---\n\n{}", persona.prompt, prompt);
        }

        // Append mode-specific suffix (e.g., Creative mode instructions)
        if let Some(suffix) = self.mode.prompt_suffix() {
            prompt.push_str(suffix);
        }

        if let Some(first) = self.messages.first_mut() {
            if first.role == anvil_llm::Role::System {
                first.content = Some(prompt);
                return;
            }
        }
        self.messages.insert(0, ChatMessage::system(prompt));
    }

    /// Access the MCP manager for slash commands and status display.
    pub fn mcp(&self) -> &Arc<McpManager> {
        &self.mcp
    }

    /// Activate a character persona. Rebuilds the system prompt with persona instructions.
    /// When a kids persona is activated and `kids_workspace` is configured,
    /// the executor's sandbox is engaged — restricting file paths and shell commands.
    pub fn set_persona(&mut self, persona: Option<crate::persona::Persona>) {
        // Update sandbox based on persona
        if let Some(ref p) = persona {
            if crate::persona::is_kids_persona(&p.key) {
                if let Some(ref kids_ws) = self.settings.agent.kids_workspace {
                    // Manual tilde expansion (no shellexpand dependency)
                    let expanded = if let Some(rest) = kids_ws.strip_prefix("~/") {
                        if let Some(home) = std::env::var_os("HOME") {
                            std::path::PathBuf::from(home).join(rest)
                        } else {
                            std::path::PathBuf::from(kids_ws)
                        }
                    } else {
                        std::path::PathBuf::from(kids_ws)
                    };
                    let ws_path = expanded;
                    // Create the directory if it doesn't exist
                    let _ = std::fs::create_dir_all(&ws_path);
                    let allowed = self
                        .settings
                        .agent
                        .kids_allowed_commands
                        .clone()
                        .unwrap_or_else(|| {
                            anvil_tools::DEFAULT_KIDS_COMMANDS
                                .iter()
                                .map(|s| s.to_string())
                                .collect()
                        });
                    self.executor.set_kids_sandbox(anvil_tools::KidsSandbox {
                        workspace: ws_path,
                        allowed_commands: allowed,
                    });
                }
            } else {
                self.executor.clear_kids_sandbox();
            }
        } else {
            self.executor.clear_kids_sandbox();
        }

        // Set mode based on persona (kids → Creative, homelab → Coding)
        self.mode = match &persona {
            Some(p) => Mode::for_persona(&p.key),
            None => Mode::default(),
        };

        self.active_persona = persona;
        self.rebuild_system_prompt();
    }

    /// Get the active persona, if any.
    pub fn persona(&self) -> Option<&crate::persona::Persona> {
        self.active_persona.as_ref()
    }

    /// Get the current operating mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Set the operating mode. Rebuilds the system prompt to include/exclude
    /// the mode-specific suffix.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.rebuild_system_prompt();
    }

    pub fn context_limit(&self) -> usize {
        self.context_limit
    }

    pub fn set_context_limit(&mut self, limit: usize) {
        self.context_limit = limit;
    }

    fn estimate_context_tokens(&self) -> usize {
        self.messages
            .iter()
            .map(|m| {
                let content_len = m.content.as_deref().map(|c| c.len()).unwrap_or(0);
                let tc_len = m
                    .tool_calls
                    .as_ref()
                    .map(|tcs| {
                        tcs.iter()
                            .map(|tc| tc.function.name.len() + tc.function.arguments.len())
                            .sum::<usize>()
                    })
                    .unwrap_or(0);
                (content_len + tc_len) / 4 // rough chars-to-tokens
            })
            .sum()
    }

    fn check_loop_detection(&self, tool_name: &str, arguments: &str) -> Option<usize> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        tool_name.hash(&mut hasher);
        arguments.hash(&mut hasher);
        let hash = hasher.finish();

        let count = self.tool_call_hashes.iter().filter(|&&h| h == hash).count();
        if count >= self.loop_detection_limit {
            Some(count)
        } else {
            None
        }
    }

    fn record_tool_call_hash(&mut self, tool_name: &str, arguments: &str) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        tool_name.hash(&mut hasher);
        arguments.hash(&mut hasher);
        let hash = hasher.finish();

        self.tool_call_hashes.push(hash);
        // Keep only last 20 entries
        if self.tool_call_hashes.len() > 20 {
            self.tool_call_hashes.remove(0);
        }
    }

    /// Run one turn of the agent loop: send user message, get response,
    /// handle tool calls, return events via channel.
    ///
    /// # Cancellation
    /// When the `cancel` token fires, the current LLM stream or tool execution
    /// is aborted. Partial content received before cancellation is saved to the
    /// session. The turn returns `Ok(())` after emitting `AgentEvent::Cancelled`.
    pub async fn turn(
        &mut self,
        user_input: &str,
        event_tx: &mpsc::Sender<AgentEvent>,
        mut permission_rx: mpsc::Receiver<PermissionDecision>,
        cancel: CancellationToken,
    ) -> Result<()> {
        let user_msg = ChatMessage::user(user_input);
        self.messages.push(user_msg);
        self.record_last_message();
        self.store
            .save_message(&self.session_id, "user", Some(user_input), None, None)?;

        // Agent loop: keep going until the assistant responds without tool calls
        loop {
            // Check cancellation before starting a new LLM call
            if cancel.is_cancelled() {
                let _ = event_tx.send(AgentEvent::Cancelled).await;
                return Ok(());
            }

            // Context window check and auto-compaction
            if self.context_limit > 0 {
                let estimated = self.estimate_context_tokens();
                let pct = (estimated * 100) / self.context_limit;

                // Auto-compact if threshold is set and exceeded
                if self.auto_compact_threshold > 0 && pct >= self.auto_compact_threshold as usize {
                    let result = self.compact(4, event_tx, cancel.clone()).await?;
                    if result.messages_removed > 0 {
                        let _ = event_tx
                            .send(AgentEvent::AutoCompacted {
                                before_tokens: result.before_tokens,
                                after_tokens: result.after_tokens,
                                messages_removed: result.messages_removed,
                            })
                            .await;
                    }
                } else if pct >= 80 {
                    let _ = event_tx
                        .send(AgentEvent::ContextWarning {
                            estimated_tokens: estimated,
                            limit: self.context_limit,
                        })
                        .await;
                }
            }

            // Build tools and tool_choice based on active mode.
            // Creative mode omits tools entirely so the model responds directly.
            // Coding mode sends all tools with tool_choice: "auto".
            // Kids personas/skills force tool_choice: "required" to ensure ACTION-FIRST behavior.
            let is_kids_mode = self.persona().map_or(false, |p| crate::persona::is_kids_persona(&p.key))
                || self.has_active_skill("kids-first")
                || self.has_active_skill("kids-game")
                || self.has_active_skill("kids-story");

            let (tools, tool_choice) = match self.mode {
                Mode::Creative => (None, Some(anvil_llm::ToolChoice::none())),
                Mode::Coding => {
                    let mut tool_defs = all_tool_definitions();
                    let mcp_defs = self.mcp.tool_definitions().await;
                    tool_defs.extend(mcp_defs);
                    let tools_json: Vec<anvil_llm::ToolDefinition> =
                        serde_json::from_value(serde_json::Value::Array(tool_defs))?;
                    let choice = if is_kids_mode {
                        anvil_llm::ToolChoice::required()
                    } else {
                        anvil_llm::ToolChoice::auto()
                    };
                    (Some(tools_json), Some(choice))
                }
            };

            let mut request = ChatRequest {
                model: String::new(), // filled by client
                messages: self.messages.clone(),
                tools,
                tool_choice,
                temperature: None,
                top_p: None,
                min_p: None,
                repeat_penalty: None,
                top_k: None,
                stream: true,
            };

            let event_tx_retry = event_tx.clone();
            let mut rx = self
                .client
                .chat_stream(&mut request, cancel.clone(), move |attempt, max, delay| {
                    let _ = event_tx_retry.try_send(AgentEvent::Retry {
                        attempt,
                        max,
                        delay_secs: delay.as_secs_f64(),
                    });
                })
                .await?;

            let mut content_buf = String::new();
            let mut tool_acc = ToolCallAccumulator::default();
            let mut _stream_usage = None;
            let mut was_cancelled = false;

            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::ContentDelta(delta) => {
                        // Store full content (including thinking) for session history
                        content_buf.push_str(&delta);

                        // Filter thinking blocks for display
                        let result = self.thinking_filter.push(&delta);
                        if !result.display.is_empty() {
                            let _ = event_tx
                                .send(AgentEvent::ContentDelta(result.display))
                                .await;
                        }
                        if !result.thinking.is_empty() {
                            let _ = event_tx
                                .send(AgentEvent::ThinkingDelta(result.thinking))
                                .await;
                        }
                    }
                    StreamEvent::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => {
                        tool_acc.push_delta(index, id, name, &arguments_delta);
                    }
                    StreamEvent::Usage(u) => {
                        self.client
                            .record_stream_usage(u.prompt_tokens, u.completion_tokens);
                        _stream_usage = Some(u);
                    }
                    StreamEvent::Error(msg) => {
                        tracing::warn!("mid-stream error: {msg}");
                        let _ = event_tx
                            .send(AgentEvent::ContentDelta(format!(
                                "\n\n[stream error: {msg}]\n"
                            )))
                            .await;
                        was_cancelled = true;
                        break;
                    }
                    StreamEvent::Done => break,
                }
            }

            // Flush any buffered thinking content
            let flush_result = self.thinking_filter.flush();
            if !flush_result.display.is_empty() {
                let _ = event_tx
                    .send(AgentEvent::ContentDelta(flush_result.display))
                    .await;
            }

            // Check if we were cancelled during streaming
            if cancel.is_cancelled() {
                was_cancelled = true;
            }

            let _ = event_tx
                .send(AgentEvent::Usage(self.client.usage().clone()))
                .await;

            // Persist cumulative usage so cost data survives restarts
            let _ = self
                .store
                .update_session_usage(&self.session_id, self.client.usage());

            let tool_calls = tool_acc.finish();

            // Save assistant message (including partial content from cancellation)
            let tc_json = if tool_calls.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&tool_calls)?)
            };
            let content_opt = if content_buf.is_empty() {
                None
            } else {
                Some(content_buf.as_str())
            };
            let msg_id = self.store.save_message(
                &self.session_id,
                "assistant",
                content_opt,
                tc_json.as_deref(),
                None,
            )?;

            // Build the assistant message for conversation history
            let mut assistant_msg = ChatMessage::assistant(&content_buf);
            if !tool_calls.is_empty() {
                assistant_msg.tool_calls = Some(tool_calls.clone());
            }
            self.messages.push(assistant_msg);
            self.record_last_message();

            // If cancelled, save partial state and exit
            if was_cancelled {
                let _ = event_tx.send(AgentEvent::Cancelled).await;
                return Ok(());
            }

            if tool_calls.is_empty() {
                // Persist agent state so resume restores mode/persona/skills
                if let Err(e) = self.persist_snapshot() {
                    tracing::warn!("failed to persist session snapshot: {e}");
                }
                let _ = event_tx.send(AgentEvent::TurnComplete).await;
                return Ok(());
            }

            // Partition tool calls: read-only built-in tools run in parallel,
            // mutating tools and MCP tools run sequentially after.
            // MCP tools are always sequential (external process I/O).
            let (read_only, mutating): (Vec<_>, Vec<_>) = tool_calls.iter().partition(|tc| {
                !McpManager::is_mcp_tool(&tc.function.name)
                    && anvil_tools::PermissionHandler::is_read_only(&tc.function.name)
            });

            // Execute read-only tools in parallel
            if !read_only.is_empty() && !cancel.is_cancelled() {
                let mut handles = Vec::new();
                for tc in &read_only {
                    self.record_tool_call_hash(&tc.function.name, &tc.function.arguments);
                    let args: serde_json::Value =
                        crate::json_filter::extract_json(&tc.function.arguments);
                    let name = tc.function.name.clone();
                    let executor = self.executor.clone();
                    handles.push(tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        let result = executor
                            .execute(&name, &args)
                            .await
                            .unwrap_or_else(|e| format!("error: {e}").into());
                        let duration = start.elapsed();
                        (result, duration)
                    }));
                }

                // Collect results in original order
                for (tc, handle) in read_only.iter().zip(handles) {
                    let (result, duration) = handle.await.unwrap_or_else(|e| {
                        (
                            format!("error: task panicked: {e}").into(),
                            std::time::Duration::ZERO,
                        )
                    });

                    let result_text = result.text().to_string();
                    self.store.save_tool_call(&ToolCallEntry {
                        session_id: &self.session_id,
                        message_id: &msg_id,
                        tool_name: &tc.function.name,
                        arguments: &tc.function.arguments,
                        result: Some(&result_text),
                        duration_ms: Some(duration.as_millis() as i64),
                        permission: "allowed",
                    })?;

                    let _ = event_tx
                        .send(AgentEvent::ToolResult {
                            name: tc.function.name.clone(),
                            result,
                        })
                        .await;

                    let tool_result_msg = ChatMessage::tool_result(&tc.id, &result_text);
                    self.messages.push(tool_result_msg);
                    self.record_last_message();
                    self.store.save_message(
                        &self.session_id,
                        "tool",
                        Some(&result_text),
                        None,
                        Some(&tc.id),
                    )?;
                }
            }

            // Execute mutating/MCP tools sequentially
            for tc in &mutating {
                if cancel.is_cancelled() {
                    let _ = event_tx.send(AgentEvent::Cancelled).await;
                    return Ok(());
                }

                let args: serde_json::Value =
                    crate::json_filter::extract_json(&tc.function.arguments);

                // Loop detection
                if let Some(count) =
                    self.check_loop_detection(&tc.function.name, &tc.function.arguments)
                {
                    let _ = event_tx
                        .send(AgentEvent::LoopDetected {
                            tool_name: tc.function.name.clone(),
                            count,
                        })
                        .await;
                }
                self.record_tool_call_hash(&tc.function.name, &tc.function.arguments);

                let is_mcp = McpManager::is_mcp_tool(&tc.function.name);

                let needs_permission = !self
                    .executor
                    .permissions()
                    .is_always_allowed(&tc.function.name);

                let decision = if needs_permission {
                    let _ = event_tx
                        .send(AgentEvent::ToolCallPending {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        })
                        .await;

                    permission_rx
                        .recv()
                        .await
                        .unwrap_or(PermissionDecision::Deny)
                } else {
                    PermissionDecision::Allow
                };

                let result: anvil_tools::ToolOutput = match decision {
                    PermissionDecision::Allow | PermissionDecision::AllowAlways => {
                        if decision == PermissionDecision::AllowAlways {
                            self.executor.permissions().grant_always(&tc.function.name);
                        }

                        let permission = if decision == PermissionDecision::AllowAlways {
                            "always_allowed"
                        } else {
                            "allowed"
                        };

                        let start = std::time::Instant::now();

                        // Dispatch to MCP or built-in executor
                        let result: anvil_tools::ToolOutput = if is_mcp {
                            self.mcp
                                .call_tool(&tc.function.name, &args)
                                .await
                                .unwrap_or_else(|e| format!("error: {e}"))
                                .into()
                        } else {
                            self.executor
                                .execute(&tc.function.name, &args)
                                .await
                                .unwrap_or_else(|e| format!("error: {e}").into())
                        };

                        let duration = start.elapsed();
                        let result_text = result.text().to_string();

                        self.store.save_tool_call(&ToolCallEntry {
                            session_id: &self.session_id,
                            message_id: &msg_id,
                            tool_name: &tc.function.name,
                            arguments: &tc.function.arguments,
                            result: Some(&result_text),
                            duration_ms: Some(duration.as_millis() as i64),
                            permission,
                        })?;

                        let _ = event_tx
                            .send(AgentEvent::ToolResult {
                                name: tc.function.name.clone(),
                                result,
                            })
                            .await;

                        // Re-create ToolOutput from text for message history
                        result_text.into()
                    }
                    PermissionDecision::Deny => {
                        self.store.save_tool_call(&ToolCallEntry {
                            session_id: &self.session_id,
                            message_id: &msg_id,
                            tool_name: &tc.function.name,
                            arguments: &tc.function.arguments,
                            result: Some("denied by user"),
                            duration_ms: None,
                            permission: "denied",
                        })?;

                        "Tool execution denied by user.".to_string().into()
                    }
                };

                let result_text = result.into_text();
                let tool_result_msg = ChatMessage::tool_result(&tc.id, &result_text);
                self.messages.push(tool_result_msg);
                self.record_last_message();
                self.store.save_message(
                    &self.session_id,
                    "tool",
                    Some(&result_text),
                    None,
                    Some(&tc.id),
                )?;
            }

            // Loop back to let the LLM respond to tool results
        }
    }
}

/// Split a message list for compaction: returns (compact_start, compact_end).
/// Messages in [compact_start..compact_end] should be summarized.
/// Returns None if there aren't enough messages to compact.
fn compaction_range(total_messages: usize, keep_recent: usize) -> Option<(usize, usize)> {
    if total_messages <= keep_recent + 1 {
        return None;
    }
    let compact_start = 1; // skip system prompt
    let compact_end = total_messages.saturating_sub(keep_recent);
    if compact_end <= compact_start {
        return None;
    }
    Some((compact_start, compact_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_range_too_few_messages() {
        // system + 3 messages, keep_recent=4 → nothing to compact
        assert_eq!(compaction_range(4, 4), None);
    }

    #[test]
    fn compaction_range_exactly_enough() {
        // system + 5 messages, keep_recent=4 → compact 1 message
        assert_eq!(compaction_range(6, 4), Some((1, 2)));
    }

    #[test]
    fn compaction_range_many_messages() {
        // system + 19 messages, keep_recent=4 → compact 15 messages
        assert_eq!(compaction_range(20, 4), Some((1, 16)));
    }

    #[test]
    fn compaction_range_keep_recent_zero() {
        // system + 5 messages, keep_recent=0 → compact all non-system
        assert_eq!(compaction_range(6, 0), Some((1, 6)));
    }

    #[test]
    fn compaction_preserves_system_and_recent() {
        // Simulate message list: [system, u1, a1, u2, a2, u3, a3]
        let messages = [
            ChatMessage::system("system prompt".to_string()),
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
            ChatMessage::user("q2"),
            ChatMessage::assistant("a2"),
            ChatMessage::user("q3"),
            ChatMessage::assistant("a3"),
        ];

        let (start, end) = compaction_range(messages.len(), 4).unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 3); // compact messages[1..3] = [u1, a1]

        // Verify preserved messages
        let preserved_before = &messages[0..1]; // system
        let preserved_after = &messages[end..]; // [u2, a2, u3, a3]
        assert_eq!(preserved_before.len(), 1);
        assert_eq!(preserved_after.len(), 4);
        assert_eq!(
            preserved_before[0].content.as_deref(),
            Some("system prompt")
        );
        assert_eq!(preserved_after[0].content.as_deref(), Some("q2"));
    }
}
