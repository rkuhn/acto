//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel (even for the SPSC API), should probably use feature flags, and still is quite slow.

mod actor;

// #[cfg(feature = "with_async-std")]
// pub mod async_std;
// pub mod thread;
#[cfg(feature = "with_tokio")]
pub mod tokio;

pub use actor::{spawn, ActorCell, ActorError, ActorRef, ActorResult, NoActorRef, Runtime};

pub mod for_implementors {
    pub use super::actor::{Dropper, Receiver, Sender, TaskFuture};
}
