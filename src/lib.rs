//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel, and still is quite slow.

#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "tokio")]
pub use crate::tokio::AcTokio;

mod actor;

pub use actor::{
    join, ActoCell, ActoHandle, ActoId, ActoInput, ActoRef, ActoRuntime, MailboxSize, Receiver,
    Sender, SupervisionRef,
};

#[cfg(test)]
mod tests;
