//! Runtime-agnostic actor library for Rust
//!
//! Currently supports [`tokio`](https://docs.rs/tokio) for execution.
//! Please refer to [`AcTokio`] for example usage.
//!
//! Actors combine well with sharing immutable snapshots, like [`futures-signals`](https://docs.rs/futures-signals).

#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]
// while acto works find with single-threaded runtimes, it must guarantee spawning on multi-threaded ones as well
#![deny(clippy::future_not_send)]

#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "tokio")]
pub use crate::tokio::{AcTokio, AcTokioRuntime};

mod actor;

pub use actor::{
    ActoAborted, ActoCell, ActoHandle, ActoId, ActoInput, ActoMsgSuper, ActoRef, ActoRuntime,
    MailboxSize, MappedActoHandle, Receiver, Sender, SupervisionRef,
};

#[cfg(test)]
mod tests;
