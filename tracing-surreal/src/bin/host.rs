use est::AnyRes;
use std::time::Duration;
use surrealdb::{
    engine::remote::ws::{Client, Ws},
    opt::auth::Root,
    Surreal,
};
use tokio::time::timeout;
use tracing_surreal::{stop::Stop, tmp::server::BuildServerDefault};

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
    let stop = Stop::builder_default(db().await?, "test").init().await?;
    let mut server = stop
        .build_server_default()
        .pusher_token("fucker")
        .start()
        .await?;
    println!("{}", server.get_local_addr());

    let grace_type = match timeout(Duration::from_secs_f64(30.0), &mut server).await {
        Err(_) => {
            println!("5s elapsed, initiating shutdown...");
            server.graceful_shutdown().await??
        }
        Ok(res) => {
            println!("routine exited");
            res??
        }
    };

    println!("{:?}", grace_type);
    Ok(())
}
