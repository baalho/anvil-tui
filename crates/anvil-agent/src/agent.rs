use crate::session::{SessionStatus, SessionStore, ToolCallEntry};
use crate::system_prompt::build_system_prompt;
use anvil_config::Settings;
use anvil_llm::{
    ChatMessage, ChatRequest, LlmClient, StreamEvent, TokenUsage, ToolCallAccumulator,
};
use anvil_tools::{all_tool_definitions, PermissionDecision, ToolExecutor};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Events emitted by the agent loop for the UI to consume.
#[derive(Debug)]
pub enum AgentEvent {
    /// Assistant is producing text content.
    ContentDelta(String),
    /// Assistant wants to call a tool — awaiting permission.
    ToolCallPending {
        id: String,
        name: String,
        arguments: String,
    },
    /// Tool execution completed.
    ToolResult { name: String, result: String },
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
    /// Error occurred.
    Error(String),
}

pub struct Agent {
    client: LlmClient,
    executor: ToolExecutor,
    store: SessionStore,
    _settings: Settings,
    _workspace: PathBuf,
    session_id: String,
    messages: Vec<ChatMessage>,
    context_limit: usize,
    loop_detection_limit: usize,
    tool_call_hashes: Vec<u64>,
    active_skills: Vec<crate::skills::Skill>,
}

impl Agent {
    pub fn new(settings: Settings, workspace: PathBuf, store: SessionStore) -> Result<Self> {
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

        Ok(Self {
            client,
            executor,
            store,
            _settings: settings,
            _workspace: workspace,
            session_id: session.id,
            messages,
            context_limit,
            loop_detection_limit,
            tool_call_hashes: Vec::new(),
            active_skills: Vec::new(),
        })
    }

    /// Resume an existing session with previously stored messages.
    pub fn resume(
        settings: Settings,
        workspace: PathBuf,
        store: SessionStore,
        session_id: &str,
        stored_messages: Vec<crate::session::StoredMessage>,
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

        // Reconstruct ChatMessages from stored messages, skipping old system messages
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

        // Mark session as active
        store.update_session_status(session_id, &SessionStatus::Active)?;

        let context_limit = settings.agent.context_window;
        let loop_detection_limit = settings.agent.loop_detection_limit as usize;

        Ok(Self {
            client,
            executor,
            store,
            _settings: settings,
            _workspace: workspace,
            session_id: session_id.to_string(),
            messages,
            context_limit,
            loop_detection_limit,
            tool_call_hashes: Vec::new(),
            active_skills: Vec::new(),
        })
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

    fn rebuild_system_prompt(&mut self) {
        let prompt = build_system_prompt(
            &self._workspace,
            self._settings.agent.system_prompt_override.as_deref(),
            self.client.model(),
            &self.active_skills,
        );
        if let Some(first) = self.messages.first_mut() {
            if first.role == anvil_llm::Role::System {
                first.content = Some(prompt);
                return;
            }
        }
        self.messages.insert(0, ChatMessage::system(prompt));
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
    pub async fn turn(
        &mut self,
        user_input: &str,
        event_tx: &mpsc::Sender<AgentEvent>,
        mut permission_rx: mpsc::Receiver<PermissionDecision>,
    ) -> Result<()> {
        let user_msg = ChatMessage::user(user_input);
        self.messages.push(user_msg);
        self.store
            .save_message(&self.session_id, "user", Some(user_input), None, None)?;

        // Agent loop: keep going until the assistant responds without tool calls
        loop {
            // Context window check
            if self.context_limit > 0 {
                let estimated = self.estimate_context_tokens();
                let pct = (estimated * 100) / self.context_limit;
                if pct >= 80 {
                    let _ = event_tx
                        .send(AgentEvent::ContextWarning {
                            estimated_tokens: estimated,
                            limit: self.context_limit,
                        })
                        .await;
                }
            }

            let tool_defs = all_tool_definitions();
            let tools_json: Vec<anvil_llm::ToolDefinition> =
                serde_json::from_value(serde_json::Value::Array(tool_defs))?;

            let mut request = ChatRequest {
                model: String::new(), // filled by client
                messages: self.messages.clone(),
                tools: Some(tools_json),
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
                .chat_stream(&mut request, move |attempt, max, delay| {
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

            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::ContentDelta(delta) => {
                        content_buf.push_str(&delta);
                        let _ = event_tx.send(AgentEvent::ContentDelta(delta)).await;
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
                    StreamEvent::Done => break,
                }
            }

            let _ = event_tx
                .send(AgentEvent::Usage(self.client.usage().clone()))
                .await;

            let tool_calls = tool_acc.finish();

            // Save assistant message
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

            if tool_calls.is_empty() {
                let _ = event_tx.send(AgentEvent::TurnComplete).await;
                return Ok(());
            }

            // Process tool calls sequentially
            for tc in &tool_calls {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();

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

                let is_read_only = anvil_tools::PermissionHandler::is_read_only(&tc.function.name);
                let needs_permission = !is_read_only
                    && !self
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

                let result = match decision {
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
                        let result = self
                            .executor
                            .execute(&tc.function.name, &args)
                            .await
                            .unwrap_or_else(|e| format!("error: {e}"));
                        let duration = start.elapsed();

                        self.store.save_tool_call(&ToolCallEntry {
                            session_id: &self.session_id,
                            message_id: &msg_id,
                            tool_name: &tc.function.name,
                            arguments: &tc.function.arguments,
                            result: Some(&result),
                            duration_ms: Some(duration.as_millis() as i64),
                            permission,
                        })?;

                        let _ = event_tx
                            .send(AgentEvent::ToolResult {
                                name: tc.function.name.clone(),
                                result: result.clone(),
                            })
                            .await;

                        result
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

                        "Tool execution denied by user.".to_string()
                    }
                };

                // Add tool result to conversation
                let tool_result_msg = ChatMessage::tool_result(&tc.id, &result);
                self.messages.push(tool_result_msg);
                self.store.save_message(
                    &self.session_id,
                    "tool",
                    Some(&result),
                    None,
                    Some(&tc.id),
                )?;
            }

            // Loop back to let the LLM respond to tool results
        }
    }
}
