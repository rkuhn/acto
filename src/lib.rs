//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel, and still is quite slow.

#![cfg_attr(doc, feature(doc_auto_cfg))]

#[cfg(feature = "tokio")]
pub mod tokio;

mod actor;

pub use actor::{
    join, ActoCell, ActoHandle, ActoId, ActoInput, ActoRef, ActoRuntime, Receiver, Sender,
    SupervisionRef,
};
