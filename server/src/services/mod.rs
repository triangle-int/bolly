pub mod chat;
pub mod embedding;

pub mod keyword_search;
// rate_limit removed — all instances are BYOK with no rate limits
pub mod browser_sessions;
pub mod companion;
pub mod companion_routine;
pub mod drops;
pub mod heartbeat;
pub mod llm;
pub mod machine_registry;
pub mod mcp;
pub mod media_text;
pub mod memory;
pub mod proactive;
#[allow(dead_code)] // Foundation for route/producer migration in #116.
pub(crate) mod resource_capability;
pub mod rhythm;
pub mod scheduler;
pub mod skills;
pub mod soul;
pub mod tool;
pub mod tools;
pub mod uploads;
pub mod vector;
pub mod workspace;

mod vector_index;

pub(crate) mod resource_access;
