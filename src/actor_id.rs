use derive_more::Display;
use std::sync::atomic::{AtomicU64, Ordering};

static TICKET: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorId(u64);

impl ActorId {
    pub(crate) fn create() -> Self {
        let ticket = TICKET.fetch_add(1, Ordering::Relaxed);
        Self(ticket)
    }
}
