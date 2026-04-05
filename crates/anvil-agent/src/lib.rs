pub mod achievements;
mod agent;
pub mod autonomous;
pub mod dispatch;
pub mod event;
pub mod harness;
mod json_filter;
pub mod memory;
pub mod mode;
pub mod persona;
pub mod projects;
pub mod repo_map;
pub mod routing;
mod session;
pub mod skills;
pub mod system_prompt;
pub mod thinking;

pub use achievements::{AchievementStore, SessionTracker};
pub use agent::{Agent, AgentEvent, CompactionResult};
pub use autonomous::{AutonomousConfig, AutonomousRunner, IterationResult};
pub use dispatch::{dispatch_event, DispatchResult};
pub use event::Event;
pub use memory::MemoryStore;
pub use mode::Mode;
pub use persona::{builtin_personas, find_persona, is_kids_persona, random_suggestions, Persona};
pub use repo_map::RepoMap;
pub use routing::ModelRouter;
pub use session::{
    SearchResult, Session, SessionSnapshot, SessionStatus, SessionStore, StoredMessage,
    ToolCallEntry,
};
pub use skills::{Skill, SkillLoader};
pub use thinking::ThinkingFilter;

// Re-export MCP types for use by the binary crate
pub use anvil_mcp::{McpManager, McpServerConfig, McpTool};
