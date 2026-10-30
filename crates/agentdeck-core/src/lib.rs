//! Registry and harness adapters shared by the agentdeck CLI, GUI and daemon.

pub mod changes;
pub mod paths;
pub mod pool;
pub mod registry;

pub use changes::{Change, apply_changes, plan_write_file};
pub use paths::Paths;
pub use pool::{PoolEntry, PoolValue};
pub use registry::{AddSpec, Harness, Instance, Registry};
