//! Unpublished end-to-end prototype: one disposable store, one loopback port.
#![forbid(unsafe_code)]
use hengscore::{
    registration::{Fault, Store},
    submission_http, web, web_http,
};
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
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: app_probe ROOT [--port PORT] [--fail-at KIND:POINT]")?,
    );
    let mut port = 0;
    let mut fault = Fault::default();
    while let Some(option) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match option.as_str() {
            "--port" => port = value.parse::<u16>()?,
            "--fail-at" => fault = Fault(Some(value)),
            _ => return Err("unknown option".into()),
        }
    }
    fault.validate()?;
    // The marker requirement and recovery guard the write-enabled prototype.
    // Never point this binary at the original backup or the read-only source.
    Store::open_nonblocking(&root)?.recover(&Fault::default())?;
    web::render(&root, "/", None).map_err(|_| "cannot read preview database")?;
    let app = submission_http::router(root.clone(), fault).merge(web_http::router(root));
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
    println!("LISTEN http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
