use actor::{spawn_actor, spawn_actor_spsc, AnySender, Mailbox, SenderGone, MPSC, SPSC};
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
    reply: AnySender<u32>,
}

// simple pong actor: respond to ping with the contained pong value
async fn pong(mut mailbox: Mailbox<SPSC<Ping>>) -> Result<(), SenderGone> {
    loop {
        let Ping { count, mut reply } = mailbox.next().await?;
        reply.send(count);
    }
}

async fn ping(mut mailbox: Mailbox<MPSC<u32>>) -> Result<(), Error> {
    // create pong actor on its own thread
    let rt = Builder::new_multi_thread().worker_threads(1).build()?;
    let mut pong = spawn_actor_spsc(&rt, pong, ());

    // loop to handle all pong messages (u32)
    loop {
        let count = mailbox.next().await?;
        if count == 0 {
            break;
        }
        pong.send(Ping {
            count: count - 1,
            reply: mailbox.me().into(),
        });
    }

    // This actor stops when receiving a pong of zero, so clean up correctly.
    // (dropping a Runtime in async context will panic)
    // This also “kills” the pong actor.
    Handle::current().spawn_blocking(|| drop(rt));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // receive supervisor results here
    let (tx, rx) = oneshot::channel();

    // run the ping actor on its own thread
    let rt = Builder::new_multi_thread().worker_threads(1).build()?;
    let ping = spawn_actor(&rt, ping, tx);

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
