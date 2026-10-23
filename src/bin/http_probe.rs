#![forbid(unsafe_code)]
use hengscore::{
    registration::{Fault, Store},
    submission_http,
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
            .ok_or("usage: http_probe ROOT [--port PORT] [--fail-at KIND:POINT]")?,
    );
    let mut port: u16 = 0;
    let mut fault = Fault::default();
    while let Some(option) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match option.as_str() {
            "--port" => port = value.parse()?,
            "--fail-at" => fault = Fault(Some(value)),
            _ => return Err("unknown option".into()),
        }
    }
    fault.validate()?;
    // Recover before accepting requests. Do not apply test faults to recovery.
    Store::open_nonblocking(&root)?.recover(&Fault::default())?;
    let app = submission_http::router(root, fault).fallback(submission_http::not_found);
    // Bind address is deliberately not configurable.
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
    println!("LISTEN http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
