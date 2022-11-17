//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel, and still is quite slow.

#![cfg_attr(all(doc, CHANNEL_NIGHTLY), feature(doc_auto_cfg))]

#[cfg(feature = "tokio")]
mod tokio;

#[cfg(feature = "tokio")]
pub use crate::tokio::ActoTokio;

mod actor;

pub use actor::{
    join, ActoCell, ActoHandle, ActoId, ActoInput, ActoRef, ActoRuntime, Receiver, Sender,
    SupervisionRef,
};

/// Given a runtime, spawn the given actor.
pub fn spawn_actor<R, M, F, Fut>(
    runtime: &R,
    name: &str,
    actor: F,
) -> SupervisionRef<M, R::ActoHandle<Fut::Output>>
where
    R: ActoRuntime,
    M: Send + 'static,
    F: FnOnce(ActoCell<M, R>) -> Fut,
    Fut: std::future::Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    let (me, join) = runtime.spawn_actor(name, actor);
    SupervisionRef { me, join }
}

#[cfg(test)]
mod tests;
