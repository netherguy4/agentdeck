//! Registry and harness adapters shared by the agentdeck CLI, GUI and daemon.

pub mod changes;
pub mod claude;
pub mod codex;
pub mod login;
pub mod paths;
pub mod pool;
pub mod processes;
pub mod registry;
pub mod t3;
pub mod usage;
pub mod wrappers;

pub use changes::{Change, apply_changes, plan_write_file, plan_write_script};
pub use paths::Paths;
pub use pool::{PoolEntry, PoolValue};
pub use registry::{AddSpec, Harness, Instance, Registry};
pub use usage::{UsageEntry, UsageKind};
