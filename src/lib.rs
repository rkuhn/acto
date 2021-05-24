//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel (even for the SPSC API), should probably use feature flags, and still is quite slow.

mod actor;

pub use actor::tokio::{Mailbox as TokioMailbox, Spawner as TokioSpawner};
pub use actor::{ActorRef, Context, Mailbox, Spawner};
pub use anyhow::Result;
