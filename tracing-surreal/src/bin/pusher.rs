use est::AnyRes;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::error};

#[tokio::main]
async fn main() -> AnyRes {
    let res = connect_async("ws://127.0.0.1:8192/pusher?111=222&token=fucker&333=444").await;
    if let Err(error::Error::Http(resp)) = &res {
        println!(
            "{}",
            String::from_utf8(resp.body().clone().unwrap()).unwrap()
        );
    }
    let (_stream, _resp) = res?;
    sleep(Duration::from_secs(10)).await;
    println!("Hello, world!");
    Ok(())
}
