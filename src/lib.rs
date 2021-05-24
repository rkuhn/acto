//! Runtime-agnostic actor library for Rust
//!
//! Currently in early alpha stage: supports tokio for execution, currently only uses the MPSC channel (even for the SPSC API), should probably use feature flags, and still is quite slow.

/// Define and spawn a new Actor
///
/// The first form allows spawning a top-level actor defined in an `async fn`:
///
/// ```
/// use acto::{actor, Context, Result, TokioMailbox, TokioSpawner};
///
/// async fn my_actor(mut ctx: Context<String>, me: String) -> Result<()> {
///     loop {
///         let msg = ctx.receive().await?;
///         println!("{} got msg: {}", me, msg);
///     }
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let (aref, join_handle) = actor!(TokioMailbox, TokioSpawner, fn my_actor(ctx, "Fred".to_owned()));
/// }
/// ```
///
/// The second form allows spawning a top-level actor defined inline:
///
/// ```
/// use acto::{actor, Context, TokioMailbox, TokioSpawner};
///
/// #[tokio::main]
/// async fn main() {
///     let me = "Barney".to_owned();
///     let (aref, join_handle) = actor!(TokioMailbox, TokioSpawner, |ctx| {
///         loop {
///             let msg: String = ctx.receive().await?;
///             println!("{} got msg: {}", me, msg);
///         }
///     });
/// }
/// ```
///
/// The third form is for spawning actors within other actors. The `ctx` variable can be named differently,
/// but it must be the name of the enclosing actor’s `Context`. It will intentionally shadow this name within
/// the child actor, to make it obvious that the parent’s `Context` cannot be used there.
///
/// ```
/// use acto::{actor, Context, Result, TokioMailbox, TokioSpawner};
///
/// async fn my_actor(mut ctx: Context<String>, me: String) -> Result<()> {
///     loop {
///         let msg = ctx.receive().await?;
///         let (aref, _join_handle) = actor!(TokioMailbox, |ctx| {
///             let msg = ctx.receive().await?;
///             println!("servant: {}", msg);
///             // just to demonstrate that when the external ActorRef is dropped, this stops
///             let _ = ctx.receive().await?;
///             println!("servant done");
///         });
///         aref.tell(format!("{} got msg: {}", me, msg));
///     }
/// }
/// ```
#[macro_export]
macro_rules! actor {
    ($mailbox:path, $spawner:path, fn $f:ident($ctx:ident$(,$arg:expr)*)) => {{
        use $crate::Spawner;
        let _spawner = ::std::sync::Arc::new($spawner);
        let $ctx = $crate::Context::new($mailbox, _spawner.clone());
        let _aref = $ctx.me();
        let fut = Box::pin($f($ctx, $($arg),*));
        (_aref, _spawner.spawn(fut))
    }};
    ($mailbox:path, $spawner:path, |$ctx:ident| $code:block) => {{
        use $crate::Spawner;
        let _spawner = ::std::sync::Arc::new($spawner);
        let mut $ctx = $crate::Context::new($mailbox, _spawner.clone());
        let _aref = $ctx.me();
        let fut = Box::pin(async move {
            $code
            Ok(())
        });
        (_aref, _spawner.spawn(fut))
    }};
    ($mailbox:path, |$ctx:ident| $code:block) => {{
        let (fut, aref) = {
            let mut $ctx = $ctx.inherit($mailbox);
            let _aref = $ctx.me();
            let fut = Box::pin(async move {
                $code
                Ok(())
            });
            (fut, _aref)
        };
        (aref, $ctx.spawn(fut))
    }}
}

mod actor;

pub mod tokio;

pub use actor::{ActorRef, Context, Mailbox, NoActorRef, Receiver, Spawner};
pub use anyhow::Result;
