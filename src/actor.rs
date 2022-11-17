use parking_lot::Mutex;
use smol_str::SmolStr;
use std::{
    any::Any,
    fmt::Debug,
    future::{poll_fn, Future},
    hash::Hash,
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

/// Every actor has an ID within the context of its [`ActoRuntime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActoId(usize);

/// A handle for sending messages to an actor.
///
/// You may freely clone or share this handle and store it in collections.
pub struct ActoRef<M>(Arc<ActoRefInner<M, dyn Sender<M>>>);

struct ActoRefInner<M, S: ?Sized> {
    id: ActoId,
    name: SmolStr,
    count: AtomicUsize,
    dead: AtomicBool,
    waker: Mutex<Option<Waker>>,
    _ph: PhantomData<M>,
    sender: S,
}

impl<T, U> PartialEq<ActoRef<U>> for ActoRef<T> {
    fn eq(&self, other: &ActoRef<U>) -> bool {
        self.0.id == other.0.id
    }
}

impl<T> Eq for ActoRef<T> {}

impl<T, U> PartialOrd<ActoRef<U>> for ActoRef<T> {
    fn partial_cmp(&self, other: &ActoRef<U>) -> Option<std::cmp::Ordering> {
        self.0.id.partial_cmp(&other.0.id)
    }
}

impl<T> Ord for ActoRef<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.id.cmp(&other.0.id)
    }
}

impl<T> Hash for ActoRef<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

impl<M> ActoRef<M> {
    /// The [`ActoId`] of the referenced actor.
    pub fn id(&self) -> ActoId {
        self.0.id
    }

    /// The actor’s given name plus ActoRuntime name and ActoId.
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// Check whether the referenced actor is in principle still ready to receive messages.
    ///
    /// Note that this is not the same as [`ActoHandle::is_finished`], which checks whether
    /// the actor’s task is done. An actor could drop its [`ActoCell`] (yielding `true` here)
    /// or it could move it to another async task (yielding `true` from `ActoHandle`).
    pub fn is_gone(&self) -> bool {
        self.0.dead.load(Ordering::Acquire)
    }
}

impl<M: Send + 'static> ActoRef<M> {
    /// Send a message to the referenced actor.
    ///
    /// The employed channel may be at its capacity bound and the target actor may
    /// already be terminated, in which cases the message is returned in an `Err`.
    pub fn send(&self, msg: M) -> Result<(), M> {
        tracing::trace!(target = ?self, "send");
        self.0.sender.send(msg)
    }
}

impl<M> Clone for ActoRef<M> {
    fn clone(&self) -> Self {
        self.0.count.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}

impl<M> Drop for ActoRef<M> {
    fn drop(&mut self) {
        if self.0.count.fetch_sub(1, Ordering::Relaxed) == 1 {
            let waker = self.0.waker.lock().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl<M> Debug for ActoRef<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActorRef({})", self.name())
    }
}

type BoxErr = Box<dyn Any + Send + 'static>;

/// The confines of an actor, and the engine that makes it work.
///
/// Every actor is provided with an `ActoCell` when it is started, which is its
/// means of interacting with other actors.
pub struct ActoCell<M: Send + 'static, R: ActoRuntime> {
    me: ActoRef<M>,
    runtime: R,
    recv: R::Receiver<M>,
    supervised: Vec<Box<dyn ActoHandle<Output = ()>>>,
    no_senders_signaled: bool,
}

impl<M: Send + 'static, R: ActoRuntime> Drop for ActoCell<M, R> {
    fn drop(&mut self) {
        for mut h in self.supervised.drain(..) {
            h.abort();
        }
        self.me.0.dead.store(true, Ordering::Release);
    }
}

impl<M: Send + 'static, R: ActoRuntime> ActoCell<M, R> {
    /// The actor’s own [`ActoRef`] handle, which it may send elsewhere to receive messages.
    pub fn me(&mut self) -> ActoRef<M> {
        self.no_senders_signaled = false;
        self.me.clone()
    }

    /// Asynchronously `.await` the reception of inputs.
    ///
    /// These may either be a message (sent via an [`ActoRef`]), the notification that all
    /// external `ActoRef`s have been dropped, or the termination notice of a supervised
    /// actor.
    pub fn recv(&mut self) -> impl Future<Output = ActoInput<M>> + '_ {
        poll_fn(|cx| {
            for idx in 0..self.supervised.len() {
                let p = self.supervised[idx].poll(cx);
                if let Poll::Ready(x) = p {
                    let handle = self.supervised.remove(idx);
                    tracing::trace!(me = ?self.me.name(), src = ?handle.name(), "supervision");
                    return Poll::Ready(ActoInput::Supervision(handle.id(), x.unwrap_err()));
                }
            }
            if let Poll::Ready(msg) = self.recv.poll(cx) {
                tracing::trace!(me = ?self.me.name(), "message");
                return Poll::Ready(ActoInput::Message(msg));
            }
            if self.me.0.count.load(Ordering::Relaxed) == 0 {
                tracing::trace!(me = ?self.me.name(), "no sender");
                if !self.no_senders_signaled {
                    self.no_senders_signaled = true;
                    return Poll::Ready(ActoInput::NoMoreSenders);
                }
            } else if !self.no_senders_signaled {
                // only install waker if we’re interested in emitting NoMoreSenders
                *self.me.0.waker.lock() = Some(cx.waker().clone());
                // re-check in case last ref was dropped between check and lock
                if self.me.0.count.load(Ordering::Relaxed) == 0 {
                    tracing::trace!(me = ?self.me.name(), "no sender");
                    self.no_senders_signaled = true;
                    return Poll::Ready(ActoInput::NoMoreSenders);
                }
            }
            tracing::trace!(me = ?self.me.name(), "Poll::Pending");
            Poll::Pending
        })
    }

    /// Create a new actor on the same [`ActoRuntime`] as the current one.
    ///
    /// ```rust
    /// use acto::{ActoCell, ActoInput, ActoRuntime};
    ///
    /// async fn actor<M: Send + 'static, R: ActoRuntime>(cell: ActoCell<M, R>) {
    ///     // spawn and forget
    ///     cell.spawn("name", |cell: ActoCell<i32, _>| async move { todo!() });
    ///     // spawn, retrieve handle, do not supervise
    ///     let a_ref = cell.spawn("super", |mut cell| async move {
    ///         if let ActoInput::Message(msg) = cell.recv().await {
    ///             cell.supervise(msg);
    ///         }
    ///     }).me;
    ///     // spawn and let some other actor supervise
    ///     let s_ref = cell.spawn("other", |cell: ActoCell<i32, _>| async move { todo!() });
    ///     a_ref.send(s_ref).ok();
    /// }
    /// ```
    pub fn spawn<T: Send + 'static, F, Fut>(
        &self,
        name: &str,
        actor: F,
    ) -> SupervisionRef<T, R::ActoHandle<Fut::Output>>
    where
        F: FnOnce(ActoCell<T, R>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (me, join) = self.runtime.spawn_actor(name, actor);
        tracing::trace!(me = ?self.me.name(), new = ?me.name(), "spawn");
        SupervisionRef { me, join }
    }

    /// Create a new actor on the same [`ActoRuntime`] as the current one and [`ActoCell::supervise`] it.
    pub fn spawn_supervised<T: Send + 'static, F, Fut>(
        &mut self,
        name: &str,
        actor: F,
    ) -> ActoRef<T>
    where
        F: FnOnce(ActoCell<T, R>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        self.supervise(self.spawn(name, actor))
    }

    /// Supervise another actor.
    ///
    /// When that actor terminates, this actor will receive [`ActoInput::Supervision`] for it.
    /// When this actor terminates, all supervised actors will be aborted.
    pub fn supervise<T, H>(&mut self, actor: SupervisionRef<T, H>) -> ActoRef<T>
    where
        T: Send + 'static,
        H: ActoHandle + Send + 'static,
        H::Output: Send + 'static,
    {
        tracing::trace!(me = ?self.me.name(), target = ?actor.me.name(), "supervise");
        self.supervised.push(Box::new(ActoHandleBox(actor.join)));
        actor.me
    }
}

/// A package of an actor’s [`ActoRef`] and [`ActoHandle`].
///
/// This is the result of [`ActoCell::spawn`] and can be passed to [`ActoCell::supervise`].
pub struct SupervisionRef<M, H> {
    pub me: ActoRef<M>,
    pub join: H,
}

/// Actor input as received with [`ActoCell::recv`].
#[derive(Debug)]
pub enum ActoInput<M> {
    /// All previously generated [`ActoRef`] handles were dropped, leaving only
    /// the one within [`ActoCell`]; the actor may wish to terminate unless it has
    /// other sources of input.
    ///
    /// Obtaining a new handle with [`ActoCell::me`] and dropping it will again
    /// generate this input.
    NoMoreSenders,
    /// A supervised actor with the given [`ActoId`] has terminated.
    ///
    /// Use downcasting to acquire the output value emitted by the actor’s Future.
    /// Depending on the runtime, the box may instead contain the value with which
    /// the actor panicked.
    Supervision(ActoId, BoxErr),
    /// A message has been received via our [`ActoRef`] handle.
    Message(M),
}

impl<M> ActoInput<M> {
    pub fn is_sender_gone(&self) -> bool {
        matches!(self, ActoInput::NoMoreSenders)
    }

    pub fn is_supervision(&self) -> bool {
        matches!(self, ActoInput::Supervision(_, _))
    }

    pub fn is_message(&self) -> bool {
        matches!(self, ActoInput::Message(_))
    }

    /// Obtain input message or supervision unless this is [`ActoInput::NoMoreSenders`].
    ///
    /// ```rust
    /// use acto::{ActoCell, ActoRuntime};
    ///
    /// async fn actor(mut cell: ActoCell<String, impl ActoRuntime>) {
    ///     while let Some(input) = cell.recv().await.into_value() {
    ///         // do something with it
    ///     }
    ///     // actor automatically stops when all senders are gone
    /// }
    /// ```
    pub fn into_value(self) -> Option<Result<M, (ActoId, BoxErr)>> {
        match self {
            ActoInput::NoMoreSenders => None,
            ActoInput::Supervision(id, res) => Some(Err((id, res))),
            ActoInput::Message(msg) => Some(Ok(msg)),
        }
    }
}

impl<M: PartialEq> PartialEq for ActoInput<M> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Supervision(l0, _), Self::Supervision(r0, _)) => l0 == r0,
            (Self::Message(l0), Self::Message(r0)) => l0 == r0,
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}

/// For implementors: the interface of a runtime for operating actors.
///
/// Cloning a runtime should be cheap, it SHOULD be using the `Arc<Inner>` pattern.
pub trait ActoRuntime: Clone + Send + Sync + 'static {
    /// The type of handle used for joining the actor’s task.
    type ActoHandle<O: Send + 'static>: ActoHandle<Output = O>;
    /// The type of sender for emitting messages towards the actor.
    type Sender<M: Send + 'static>: Sender<M>;
    /// The type of receiver for obtaining messages sent towards the actor.
    type Receiver<M: Send + 'static>: Receiver<M>;

    /// A name for this runtime, used mainly in logging.
    fn name(&self) -> &str;

    /// The next ID to be assigned to a fresh actor.
    fn next_id(&self) -> usize;

    /// Create a new pair of sender and receiver for a fresh actor.
    fn mailbox<M: Send + 'static>(&self) -> (Self::Sender<M>, Self::Receiver<M>);

    /// Spawn an actor’s task to be driven independently and return an [`ActoHandle`]
    /// to abort or join it.
    fn spawn_task<T>(&self, id: ActoId, name: SmolStr, task: T) -> Self::ActoHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static;

    /// Provided function for spawning actors.
    ///
    /// Uses the above utilities and cannot be implemented by downstream crates.
    fn spawn_actor<T, F, Fut>(
        &self,
        name: &str,
        actor: F,
    ) -> (ActoRef<T>, Self::ActoHandle<Fut::Output>)
    where
        T: Send + 'static,
        F: FnOnce(ActoCell<T, Self>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (sender, recv) = self.mailbox();
        let id = ActoId(self.next_id());
        let mut id_str = [0u8; 16];
        let id_str = write_id(&mut id_str, id);
        let name = [name, "(", self.name(), "/", id_str, ")"]
            .into_iter()
            .collect::<SmolStr>();
        let name2 = name.clone();
        let inner = ActoRefInner {
            id,
            name,
            count: AtomicUsize::new(0),
            dead: AtomicBool::new(false),
            waker: Mutex::new(None),
            _ph: PhantomData,
            sender,
        };
        let me = ActoRef(Arc::new(inner));
        let ctx = ActoCell {
            me: me.clone(),
            runtime: self.clone(),
            recv,
            supervised: vec![],
            no_senders_signaled: false,
        };
        let handle = self.spawn_task(id, name2, (actor)(ctx));
        (me, handle)
    }
}

fn write_id(buf: &mut [u8; 16], id: ActoId) -> &str {
    let id = id.0;
    if id == 0 {
        return "0";
    }
    let mut written = 0;
    let mut shift = (16 - (id.leading_zeros()) / 4) * 4;
    while shift != 0 {
        shift -= 4;
        const HEX: [u8; 16] = *b"0123456789abcdef";
        buf[written] = HEX[(id >> shift) & 15];
        written += 1;
    }
    unsafe { std::str::from_utf8_unchecked(&buf[..written]) }
}

#[test]
fn test_write_id() {
    let mut buf = [0u8; 16];
    assert_eq!(write_id(&mut buf, ActoId(0)), "0");
    assert_eq!(write_id(&mut buf, ActoId(1)), "1");
    assert_eq!(write_id(&mut buf, ActoId(10)), "a");
    assert_eq!(write_id(&mut buf, ActoId(100)), "64");
}

/// A named closure for sending messages to a given actor.
///
/// This type is used between a runtime implementation and `acto`.
pub trait Sender<M>: Send + Sync + 'static {
    fn send(&self, msg: M) -> Result<(), M>;
}

/// A named closure for receiving messages at a given actor.
///
/// This type is used between a runtime implementation and `acto`.
pub trait Receiver<M>: Send + 'static {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<M>;
}

/// A handle for aborting or joining a running actor.
pub trait ActoHandle: Unpin + Send + Sync + 'static {
    type Output;

    /// The ID of the underlying actor.
    fn id(&self) -> ActoId;

    /// The name of the underlying actor.
    fn name(&self) -> &str;

    /// Abort the actor’s task.
    ///
    /// Behaviour is undefined if the actor is not [cancellation safe].
    ///
    /// [cancellation safe]: https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety
    fn abort(&mut self);

    /// Check whether the actor’s task is no longer running
    ///
    /// This may be the case even after [`ActoHandle::abort`] has returned, since task
    /// termination may be asynchronous.
    ///
    /// Note that this is not the same as [`ActoRef::is_gone`], which checks whether
    /// the actor is still capable of receiving messages. An actor could drop its
    /// [`ActoCell`] (yielding `true` there) or it could move it to another async task
    /// (yielding `true` here).
    fn is_finished(&mut self) -> bool;

    /// Poll this handle for whether the actor is now terminated.
    ///
    /// This method has [`Future`] semantics.
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<Self::Output, BoxErr>>;
}

/// A future for awaiting the termination of the actor underlying the given handle.
pub fn join<J: ActoHandle>(handle: J) -> impl Future<Output = Result<J::Output, BoxErr>> {
    ActoHandleFuture(handle)
}

struct ActoHandleFuture<J>(J);
impl<J: ActoHandle> Future for ActoHandleFuture<J> {
    type Output = Result<J::Output, BoxErr>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        tracing::trace!(join = ?self.as_ref().0.name(), "poll");
        self.get_mut().0.poll(cx)
    }
}

struct ActoHandleBox<J: ActoHandle>(J);
impl<J> ActoHandle for ActoHandleBox<J>
where
    J: ActoHandle,
    J::Output: Send + 'static,
{
    type Output = ();

    fn id(&self) -> ActoId {
        self.0.id()
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn abort(&mut self) {
        self.0.abort()
    }

    fn is_finished(&mut self) -> bool {
        self.0.is_finished()
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), BoxErr>> {
        self.0
            .poll(cx)
            .map(|r| r.and_then(|x| Err(Box::new(x) as BoxErr)))
    }
}
