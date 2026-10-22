//! Managed loopback-only service. No fault injection or automatic provisioning.
#![forbid(unsafe_code)]
use hengscore_poc::{
    registration::{Fault, Store},
    submission_http, web, web_http,
};
use std::{
    env,
    error::Error,
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};
use tokio::net::TcpListener;

#[cfg(unix)]
fn notify(message: &str) -> io::Result<()> {
    use std::os::unix::{
        ffi::OsStrExt,
        net::{SocketAddr, UnixDatagram},
    };
    let Some(path) = env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let sock = UnixDatagram::unbound()?;
    let bytes = path.as_os_str().as_bytes();
    if bytes.first() == Some(&b'@') {
        #[cfg(target_os = "linux")]
        {
            use std::os::linux::net::SocketAddrExt;
            sock.connect_addr(&SocketAddr::from_abstract_name(&bytes[1..])?)?;
        }
        #[cfg(not(target_os = "linux"))]
        return Err(io::Error::other("abstract notify socket requires Linux"));
    } else {
        sock.connect(PathBuf::from(path))?;
    }
    sock.send(message.as_bytes())?;
    Ok(())
}
#[cfg(not(unix))]
fn notify(_message: &str) -> io::Result<()> {
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let mut root = None;
    let mut id = None;
    let mut port = None;
    while let Some(key) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match key.as_str() {
            "--store" if root.is_none() => root = Some(PathBuf::from(value)),
            "--store-id" if id.is_none() => id = Some(value),
            "--port" if port.is_none() => port = Some(value.parse::<u16>()?),
            _ => return Err("unknown or duplicate option".into()),
        }
    }
    let root = root.ok_or("--store is required")?;
    let id = id.ok_or("--store-id is required")?;
    let port = port.ok_or("--port is required")?;
    if !root.is_absolute() {
        return Err("store path must be absolute".into());
    }
    let recovered = Store::open_managed_nonblocking(&root, &id)?.recover(&Fault::default())?;
    web::render(&root, "/", None).map_err(|_| "cannot read managed store")?;
    let app = submission_http::managed_router(root.clone(), id).merge(web_http::router(root));
    // Bind address is fixed in code; no public address option exists.
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let shutdown = async move {
        #[cfg(unix)]
        tokio::select! {_=terminate.recv()=>{},_=interrupt.recv()=>{}}
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        let _ = notify("STOPPING=1\nSTATUS=Draining requests");
        eprintln!("service_stopping graceful=true");
    };
    println!("LISTEN http://{}", listener.local_addr()?);
    eprintln!("service_ready recovered={recovered}");
    notify("READY=1\nSTATUS=Ready; startup recovery completed")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    eprintln!("service_stopped clean=true");
    Ok(())
}
