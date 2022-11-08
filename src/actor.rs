use parking_lot::Mutex;
use std::{
    any::Any,
    fmt::{Debug, Display, Write},
    future::{poll_fn, Future},
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ActorId(usize);

impl ActorId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }
}

pub struct ActorRef<M>(Arc<ActorRefInner<M, dyn Sender<M>>>);

struct ActorRefInner<M, S: ?Sized> {
    id: ActorId,
    count: AtomicUsize,
    waker: Mutex<Option<Waker>>,
    _ph: PhantomData<M>,
    sender: S,
}

impl<M> ActorRef<M> {
    pub fn id(&self) -> ActorId {
        self.0.id
    }
}

impl<M: Send + 'static> ActorRef<M> {
    pub fn send(&self, msg: M) -> Result<(), M> {
        tracing::trace!(target = ?self, "send");
        self.0.sender.send(msg)
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        self.0.count.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}

impl<M> Drop for ActorRef<M> {
    fn drop(&mut self) {
        if self.0.count.fetch_sub(1, Ordering::Relaxed) == 1 {
            let waker = self.0.waker.lock().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl<M> Debug for ActorRef<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ActorRef").field(&self.0.id.0).finish()
    }
}

impl<M: Named> Display for ActorRef<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ActorRef<")?;
        M::fmt(f)?;
        write!(f, ">({})", self.0.id.0)?;
        Ok(())
    }
}

pub trait Named {
    fn fmt(f: &mut impl Write) -> std::fmt::Result;
}

pub struct Cell<M: Send + 'static, R: ActoRuntime> {
    me: ActorRef<M>,
    runtime: R,
    recv: R::Receiver<M>,
    supervised: Vec<Box<dyn JoinHandle<Output = ()>>>,
    no_senders_signaled: bool,
}

impl<M: Send + 'static, R: ActoRuntime> Cell<M, R> {
    pub fn me(&mut self) -> ActorRef<M> {
        self.no_senders_signaled = false;
        self.me.clone()
    }

    pub fn recv(&mut self) -> impl Future<Output = RecvResult<M>> + '_ {
        poll_fn(|cx| {
            for idx in 0..self.supervised.len() {
                let p = self.supervised[idx].poll(cx);
                if let Poll::Ready(x) = p {
                    let handle = self.supervised.remove(idx);
                    tracing::trace!(me = ?self.me.id(), src = ?handle.id(), "supervision");
                    return Poll::Ready(RecvResult::Supervision(handle.id(), x.unwrap_err()));
                }
            }
            if let Poll::Ready(msg) = self.recv.poll(cx) {
                tracing::trace!(me = ?self.me.id(), "message");
                return Poll::Ready(RecvResult::Message(msg));
            }
            if self.me.0.count.load(Ordering::Relaxed) == 0 {
                tracing::trace!(me = ?self.me.id(), "no sender");
                if !self.no_senders_signaled {
                    self.no_senders_signaled = true;
                    return Poll::Ready(RecvResult::NoMoreSenders);
                }
            } else if !self.no_senders_signaled {
                // only install waker if we’re interested in emitting NoMoreSenders
                *self.me.0.waker.lock() = Some(cx.waker().clone());
                // re-check in case last ref was dropped between check and lock
                if self.me.0.count.load(Ordering::Relaxed) == 0 {
                    tracing::trace!(me = ?self.me.id(), "no sender");
                    self.no_senders_signaled = true;
                    return Poll::Ready(RecvResult::NoMoreSenders);
                }
            }
            tracing::trace!(me = ?self.me.id(), "nothing");
            Poll::Pending
        })
    }

    pub fn spawn<T: Send + 'static, F, Fut>(&self, actor: F) -> SupervisedRef<T, R, Fut::Output>
    where
        F: FnOnce(Cell<T, R>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (me, join) = self.runtime.spawn_actor(actor);
        tracing::trace!(me = ?self.me.id(), new = ?me.id(), "spawn");
        SupervisedRef { me, join }
    }

    pub fn spawn_supervised<T: Send + 'static, F, Fut>(&mut self, actor: F) -> ActorRef<T>
    where
        F: FnOnce(Cell<T, R>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        self.supervise(self.spawn(actor))
    }

    pub fn supervise<T: Send + 'static, R2: ActoRuntime, O: Send + 'static>(
        &mut self,
        actor: SupervisedRef<T, R2, O>,
    ) -> ActorRef<T> {
        tracing::trace!(me = ?self.me.id(), target = ?actor.me.id(), "supervise");
        self.supervised.push(Box::new(JoinHandleBox(actor.join)));
        actor.me
    }
}

pub struct SupervisedRef<M, R: ActoRuntime, O: Send + 'static> {
    pub me: ActorRef<M>,
    pub join: R::JoinHandle<O>,
}

#[derive(Debug)]
pub enum RecvResult<M> {
    NoMoreSenders,
    Supervision(ActorId, Box<dyn Any + Send + 'static>),
    Message(M),
}

pub trait ActoRuntime: Clone + Send + Sync + 'static {
    type JoinHandle<O: Send + 'static>: JoinHandle<Output = O>;
    type Sender<M: Send + 'static>: Sender<M>;
    type Receiver<M: Send + 'static>: Receiver<M>;

    fn fmt(&self, f: &mut impl Write) -> std::fmt::Result;

    fn next_id(&self) -> ActorId;

    fn mailbox<M: Send + 'static>(&self) -> (Self::Sender<M>, Self::Receiver<M>);

    fn spawn_task<T>(&self, id: ActorId, task: T) -> Self::JoinHandle<T::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static;

    fn spawn_actor<T, F, Fut>(&self, actor: F) -> (ActorRef<T>, Self::JoinHandle<Fut::Output>)
    where
        T: Send + 'static,
        F: FnOnce(Cell<T, Self>) -> Fut,
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (sender, recv) = self.mailbox();
        let id = self.next_id();
        let inner = ActorRefInner {
            id,
            count: AtomicUsize::new(0),
            waker: Mutex::new(None),
            _ph: PhantomData,
            sender,
        };
        let me = ActorRef(Arc::new(inner));
        let ctx = Cell {
            me: me.clone(),
            runtime: self.clone(),
            recv,
            supervised: vec![],
            no_senders_signaled: false,
        };
        let handle = self.spawn_task(id, (actor)(ctx));
        (me, handle)
    }
}

pub trait Sender<M>: Send + Sync + 'static {
    fn send(&self, msg: M) -> Result<(), M>;
}

pub trait Receiver<M>: Send + 'static {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<M>;
}

pub trait JoinHandle: Unpin + Send + 'static {
    type Output;
    fn id(&self) -> ActorId;
    fn abort(self);
    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Output, Box<dyn Any + Send + 'static>>>;
}

pub fn join<J: JoinHandle>(
    handle: J,
) -> impl Future<Output = Result<J::Output, Box<dyn Any + Send + 'static>>> {
    JoinHandleFuture(handle)
}

struct JoinHandleFuture<J>(J);
impl<J: JoinHandle> Future for JoinHandleFuture<J> {
    type Output = Result<J::Output, Box<dyn Any + Send + 'static>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        tracing::trace!(join = ?self.as_ref().0.id(), "poll");
        self.get_mut().0.poll(cx)
    }
}

struct JoinHandleBox<J: JoinHandle>(J);
impl<J> JoinHandle for JoinHandleBox<J>
where
    J: JoinHandle,
    J::Output: Send + 'static,
{
    type Output = ();

    fn id(&self) -> ActorId {
        self.0.id()
    }

    fn abort(self) {
        self.0.abort()
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Box<dyn Any + Send + 'static>>> {
        self.0
            .poll(cx)
            .map(|r| r.and_then(|x| Err(Box::new(x) as Box<dyn Any + Send + 'static>)))
    }
}
