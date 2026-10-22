#![forbid(unsafe_code)]
use hengscore_poc::{web, web_http};
use std::{
    env,
    error::Error,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};
use tokio::net::TcpListener;
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let root = PathBuf::from(args.next().ok_or("usage: view_probe ROOT [--port PORT]")?);
    let mut port = 0;
    while let Some(arg) = args.next() {
        if arg != "--port" {
            return Err("unknown option".into());
        }
        port = args.next().ok_or("missing port")?.parse::<u16>()?;
    }
    let _ = web::render(&root, "/", None).map_err(|_| "cannot read preview database")?;
    let app = web_http::router(root);
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
    println!("LISTEN http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
