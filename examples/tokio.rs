use acto::{join, tokio::TokioActor, ActoRuntime, Cell, RecvResult};
use tokio::runtime::Runtime;

async fn actor(mut ctx: Cell<i32, impl ActoRuntime>) {
    println!("main actor started");
    while let RecvResult::Message(m) = ctx.recv().await {
        ctx.spawn_supervised(|mut ctx| async move {
            println!("spawned actor for {:?}", ctx.recv().await);
        })
        .send(m)
        .unwrap();

        let r = ctx.spawn(|mut ctx| async move {
            match ctx.recv().await {
                RecvResult::NoMoreSenders => "no send".to_owned(),
                RecvResult::Supervision(_, _) => unreachable!(),
                RecvResult::Message(m) => {
                    println!("received {}", m);
                    "send".to_owned()
                }
            }
        });
        r.me.send(5 * m).unwrap();
        let result = join(r.join).await;
        println!("actor result: {:?}", result);
    }
}

fn main() {
    let rt = Runtime::new().unwrap();
    let system = TokioActor::new(rt.handle(), "theMain");
    let (r, j) = system.spawn_actor(actor);
    r.send(1).unwrap();
    r.send(2).unwrap();
    let x = rt.block_on(join(j));
    println!("result: {:?}", x);
}
