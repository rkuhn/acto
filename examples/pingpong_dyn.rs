use acto::{
    spawn::OwnThread, Actor, DynSender, Mailbox, MpQueue, Queue, Sender, SenderGone, MPSC, SPSC,
};
use derive_more::From;
use std::{sync::mpsc::RecvError, time::Instant};
use tokio::{
    runtime::{Builder, Handle},
    sync::oneshot,
};

#[derive(Debug, From)]
enum Error {
    E(std::io::Error),
    S(SenderGone),
    R(RecvError),
    T(tokio::sync::oneshot::error::RecvError),
}

struct Ping {
    count: u32,
    // senders statically distinguish SPSC/MPSC, but we don’t care and check at runtime
    reply: DynSender<u32>,
}

// simple pong actor: respond to ping with the contained pong value
async fn pong<Q: Queue<Msg = Ping>>(mut mailbox: Mailbox<Q>) -> Result<(), SenderGone> {
    loop {
        let Ping { count, mut reply } = mailbox.next().await?;
        reply.send_mut(count);
    }
}

async fn ping<Q>(mut mailbox: Mailbox<Q>) -> Result<(), Error>
where
    // any mailbox that yields u32 and can give me a sender that can be wrapped into DynSender
    // (maybe type aliases can make this nicer in the future)
    Q: MpQueue<Msg = u32>,
    Sender<Q>: Into<DynSender<Q::Msg>>,
{
    // create pong actor on its own thread
    let mut pong = Actor::spawn(&OwnThread, pong::<SPSC<Ping>>, ());

    // loop to handle all pong messages (u32)
    loop {
        let count = mailbox.next().await?;
        if count == 0 {
            break;
        }
        pong.send_mut(Ping {
            count: count - 1,
            reply: mailbox.me().into(),
        });
    }

    // This actor stops when receiving a pong of zero, so clean up correctly.
    // Both pong actor threads stop when their Senders are dropped (not yet implemented).
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // receive supervisor results here
    let (tx, rx) = oneshot::channel();

    // run the ping actor on its own thread
    let rt = Builder::new_multi_thread().worker_threads(1).build()?;
    let ping = Actor::spawn::<MPSC<u32>>(&rt, ping, tx);

    // inject some pinging rounds
    let start = Instant::now();
    ping.send(1000000);
    ping.send(1000000);
    ping.send(1000000);
    ping.send(1000000);

    // wait on supervisor channel — the result of the `ping` actor’s function body
    rx.await??;

    // dropping a runtime is “not allowed” in an async function (wat)
    Handle::current().spawn_blocking(|| drop(rt));

    println!("elapsed: {:?}", start.elapsed());
    Ok(())
}
