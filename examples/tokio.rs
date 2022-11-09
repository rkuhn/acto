use acto::{join, tokio::ActoTokio, ActoCell, ActoInput, ActoRuntime};
use tokio::runtime::Runtime;

async fn actor(mut ctx: ActoCell<i32, impl ActoRuntime>) {
    println!("main actor started");
    while let ActoInput::Message(m) = ctx.recv().await {
        ctx.spawn_supervised(|mut ctx| async move {
            println!("spawned actor for {:?}", ctx.recv().await);
        })
        .send(m)
        .unwrap();

        let r = ctx.spawn(|mut ctx| async move {
            match ctx.recv().await {
                ActoInput::NoMoreSenders => "no send".to_owned(),
                ActoInput::Supervision(_, _) => unreachable!(),
                ActoInput::Message(m) => {
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
    let system = ActoTokio::new(rt.handle(), "theMain");
    let (r, j) = system.spawn_actor(actor);
    r.send(1).unwrap();
    r.send(2).unwrap();
    let x = rt.block_on(join(j));
    println!("result: {:?}", x);
}
