use acto::{join, ActoCell, ActoInput, ActoRuntime, ActoTokio};

async fn actor(mut ctx: ActoCell<i32, impl ActoRuntime>) {
    println!("main actor started");
    while let ActoInput::Message(m) = ctx.recv().await {
        ctx.spawn_supervised("subordinate", |mut ctx| async move {
            println!("spawned actor for {:?}", ctx.recv().await);
        })
        .send(m);

        let r = ctx.spawn("worker", |mut ctx| async move {
            match ctx.recv().await {
                ActoInput::NoMoreSenders => "no send".to_owned(),
                ActoInput::Supervision(_, _) => unreachable!(),
                ActoInput::Message(m) => {
                    println!("received {}", m);
                    "send".to_owned()
                }
            }
        });
        r.me.send(5 * m);
        let result = join(r.join).await;
        println!("actor result: {:?}", result);
    }
}

fn main() {
    let system = ActoTokio::new("theMain", 2).unwrap();
    let (r, j) = system.spawn_actor("supervisor", actor);
    r.send(1);
    r.send(2);
    let x = system.rt().block_on(join(j));
    println!("result: {:?}", x);
}
