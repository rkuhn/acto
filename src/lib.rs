//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel (even for the SPSC API), should probably use feature flags, and still is quite slow.

#[cfg(feature = "tokio")]
pub mod tokio;

mod actor;

pub use actor::{
    join, ActoRuntime, ActorId, ActorRef, Cell, JoinHandle, Named, Receiver, RecvResult, Sender,
    SupervisedRef,
};
