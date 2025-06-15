//! Runtime-agnostic actor library for Rust
//!
//! Currently supports [`tokio`](https://docs.rs/tokio) for execution.
//! Please refer to [`AcTokio`] for example usage.
//!
//! Actors combine well with sharing immutable snapshots, like [`futures-signals`](https://docs.rs/futures-signals).

// while acto works fine with single-threaded runtimes, it must guarantee spawning on multi-threaded ones as well
#![deny(clippy::future_not_send)]

#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "tokio")]
pub use crate::tokio::{AcTokio, AcTokioRuntime, TokioJoinHandle};

mod actor;
pub mod variable;

pub use actor::{
    ActoAborted, ActoCell, ActoHandle, ActoId, ActoInput, ActoMsgSuper, ActoRef, ActoRuntime,
    MailboxSize, MappedActoHandle, PanicInfo, PanicOrAbort, Receiver, Sender, SupervisionRef,
};

#[cfg(test)]
mod tests;
