pub mod achievements;
mod agent;
pub mod autonomous;
pub mod memory;
pub mod persona;
pub mod routing;
mod session;
pub mod skills;
mod system_prompt;
pub mod thinking;

pub use achievements::{AchievementStore, SessionTracker};
pub use agent::{Agent, AgentEvent, CompactionResult};
pub use autonomous::{AutonomousConfig, AutonomousRunner, IterationResult};
pub use memory::MemoryStore;
pub use persona::{builtin_personas, find_persona, Persona};
pub use routing::ModelRouter;
pub use session::{
    SearchResult, Session, SessionStatus, SessionStore, StoredMessage, ToolCallEntry,
};
pub use skills::{Skill, SkillLoader};
pub use thinking::ThinkingFilter;

// Re-export MCP types for use by the binary crate
pub use anvil_mcp::{McpManager, McpServerConfig, McpTool};
