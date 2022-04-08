use futures::{future::BoxFuture, FutureExt};
use parking_lot::Mutex;
use std::{
    error::Error,
    future::Future,
    marker::PhantomData,
    mem::{size_of, MaybeUninit},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
};

pub fn spawn<F, M, Fut>(runtime: impl Runtime, actor: F) -> ActorRef<M>
where
    F: FnOnce(ActorCell<M>) -> Fut,
    Fut: Future<Output = ActorResult> + Send + 'static,
    M: Unpin + Send + 'static,
{
    let (tell, recv) = runtime.new_actor(size_of::<M>(), unsafe {
        std::mem::transmute(std::ptr::drop_in_place::<M> as unsafe fn(*mut M))
    });
    let runtime: Arc<dyn Runtime> = Arc::new(runtime);
    let aref = ActorRef::new(tell);
    let ctx = ActorCell::new(recv, aref.clone(), runtime.clone());
    runtime.spawn(actor(ctx).boxed());
    aref
}

/// An ActorRef is the sending side of the actor’s mailbox, it can be freely cloned.
///
/// When the last `ActorRef` for an actor has been dropped, only the actor itself can
/// fabricate a new one. And if the actor is waiting for new messages from its mailbox
/// before a new `ActorRef` is created, the actor would be stuck in this state forever.
/// Therefore, the actor’s `receive()` function returns an error in this case, which
/// will usually lead to the actor shutting down.
pub struct ActorRef<M>(Arc<ActorRefInner>, PhantomData<M>);

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        self.0.count.fetch_add(1, Ordering::SeqCst);
        Self(self.0.clone(), PhantomData)
    }
}

impl<M> Drop for ActorRef<M> {
    fn drop(&mut self) {
        let prev = self.0.count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // we’re the last external reference, so wake up the Actor if it is waiting for us
            if let Some(waker) = self.0.waker.lock().take() {
                waker.wake();
            }
        }
    }
}

impl<M> ActorRef<M> {
    fn new(tell: Sender) -> Self {
        Self(
            Arc::new(ActorRefInner {
                count: AtomicUsize::new(0),
                waker: Mutex::new(None),
                tell,
                dropped: AtomicBool::new(false),
            }),
            PhantomData,
        )
    }

    pub fn tell(&self, msg: M) {
        let bytes = &msg as *const _ as *const u8;
        std::mem::forget(msg);
        (self.0.tell)(bytes);
    }

    pub fn is_dead(&self) -> bool {
        self.0.dropped.load(Ordering::Acquire)
    }
}

struct ActorRefInner {
    count: AtomicUsize,
    waker: Mutex<Option<Waker>>,
    tell: Box<dyn Fn(*const u8) + Send + Sync + 'static>,
    dropped: AtomicBool,
}

pub type Dropper = unsafe fn(*mut u8);
pub type Sender = Box<dyn Fn(*const u8) + Send + Sync + 'static>;
pub type Receiver = Box<dyn FnMut(*mut u8, &mut Context<'_>) -> Poll<()> + Send + 'static>;
pub type TaskFuture = BoxFuture<'static, ActorResult>;
pub type ActorResult = Result<(), ActorError>;

pub enum ActorError {
    NoActorRef,
    Other(Box<dyn Error + Send + 'static>),
}

impl From<NoActorRef> for ActorError {
    fn from(_: NoActorRef) -> Self {
        ActorError::NoActorRef
    }
}

impl<E: Error + Send + 'static> From<E> for ActorError {
    fn from(e: E) -> Self {
        ActorError::Other(Box::new(e))
    }
}

pub trait Runtime: Send + Sync + 'static {
    fn new_actor(&self, msg_size: usize, dropper: Dropper) -> (Sender, Receiver);
    fn spawn(&self, task: TaskFuture);
}

/// The context in which an actor is running
///
/// This context allows the actor to
///
/// - receive messages
/// - get its own address (i.e. [`ActorRef`](struct.ActorRef.html))
/// - spawn new actors
pub struct ActorCell<M> {
    recv: Receiver,
    aref: ActorRef<M>,
    runtime: Arc<dyn Runtime>,
}

impl<M> Drop for ActorCell<M> {
    fn drop(&mut self) {
        self.aref.0.dropped.store(true, Ordering::Release);
    }
}

impl<M: Unpin + Send + 'static> ActorCell<M> {
    pub fn new(recv: Receiver, aref: ActorRef<M>, runtime: Arc<dyn Runtime>) -> Self {
        Self {
            recv,
            aref,
            runtime,
        }
    }

    /// Receive the next message from the mailbox, possibly waiting for it
    ///
    /// This method’s return value should always be immediately `.await`ed, it has no other use.
    pub fn receive(&mut self) -> impl Future<Output = Result<M, NoActorRef>> + '_ {
        ReceiveFuture {
            aref: &self.aref.0,
            poll: &mut self.recv,
            _ph: PhantomData,
        }
    }

    /// The actors own address a.k.a. ActorRef
    ///
    /// The actor can put this `ActorRef` into messages to send it to other actors so that they
    /// can contact this actor back later.
    pub fn me(&self) -> ActorRef<M> {
        self.aref.clone()
    }

    /// Spawn a future for the purpose of running a child actor
    ///
    /// This method is best used via the [`actor`](macro.actor.html) macro.
    pub fn spawn<F, M2, Fut>(&self, actor: F) -> ActorRef<M2>
    where
        F: FnOnce(ActorCell<M2>) -> Fut,
        Fut: Future<Output = ActorResult> + Send + 'static,
        M2: Unpin + Send + 'static,
    {
        let (aref, recv) = self.runtime.new_actor(size_of::<M2>(), unsafe {
            std::mem::transmute(std::ptr::drop_in_place::<M2> as unsafe fn(*mut M2))
        });
        let aref = ActorRef::new(aref);
        let ctx = ActorCell::new(recv, aref.clone(), self.runtime.clone());
        self.runtime.spawn(actor(ctx).boxed());
        aref
    }
}

struct ReceiveFuture<'a, M> {
    aref: &'a ActorRefInner,
    poll: &'a mut (dyn for<'b, 'c> FnMut(*mut u8, &'b mut Context<'c>) -> Poll<()> + Send + 'a),
    _ph: PhantomData<M>,
}

impl<'a, M: Unpin + Send + 'static> Future for ReceiveFuture<'a, M> {
    type Output = Result<M, NoActorRef>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut msg = MaybeUninit::uninit();
        match (this.poll)(msg.as_mut_ptr() as *mut u8, cx) {
            Poll::Ready(_) => Poll::Ready(Ok(unsafe { msg.assume_init() })),
            Poll::Pending => {
                if this.aref.count.load(Ordering::SeqCst) == 0 {
                    // observing no external address means that there cannot be one created, either:
                    // we hold an exclusive reference on the Context (via this.poll)
                    Poll::Ready(Err(NoActorRef))
                } else {
                    *this.aref.waker.lock() = Some(cx.waker().clone());
                    // in case the last ActorRef was dropped between the check and installing the waker,
                    // we must now re-check
                    if this.aref.count.load(Ordering::SeqCst) == 0 {
                        Poll::Ready(Err(NoActorRef))
                    } else {
                        Poll::Pending
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct NoActorRef;
