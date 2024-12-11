use est::AnyRes;
use std::time::Duration;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};
use tokio::time::sleep;
use tracing_surreal::{stop::Stop, tmp::server::ServerBuilder};

async fn db() -> AnyRes<Surreal<Client>> {
    let db = Surreal::new::<Ws>("localhost:8000").await?;

    db.signin(Root {
        username: "root",
        password: "root",
    })
    .await?;
    db.use_ns("root").await?;
    Ok(db)
}

#[tokio::main]
async fn main() -> AnyRes {
    let stop = Stop::init(db().await?, "test").await?;
    let mut server = ServerBuilder::from_stop_default(&stop).start().await?;
    println!("{}", server.get_local_addr());

    let grace_type = tokio::select! {
        _ = sleep(Duration::from_secs_f64(5.0)) => {
            println!("5s elapsed, initiating shutdown...");
            server.graceful_shutdown().await??
        }
        res = server.get_routine_mut() => {
            println!("routine exited");
            res??
        }
    };

    println!("{:?}", grace_type);
    Ok(())
}
