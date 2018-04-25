extern crate tokio;
extern crate tokio_timer;
extern crate quicr;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate futures;
extern crate url;

use std::net::{UdpSocket, ToSocketAddrs};
use std::io::{self, Write};

use futures::Future;
use tokio::executor::current_thread::CurrentThread;
use url::Url;

use slog::{Logger, Drain};
use failure::Error;

type Result<T> = std::result::Result<T, Error>;

fn main() {
    let code = {
        let decorator = slog_term::PlainSyncDecorator::new(std::io::stderr());
        let drain = slog_term::FullFormat::new(decorator).use_original_order().build().fuse();
        if let Err(e) = run(Logger::root(drain, o!())) {
            eprintln!("ERROR: {}", e);
            1
        } else { 0 }
    };
    ::std::process::exit(code);
}

fn run(log: Logger) -> Result<()> {
    let url = Url::parse(&::std::env::args().nth(1).ok_or(format_err!("missing address argument"))?)?;
    let remote = url.with_default_port(|_| Ok(4433))?.to_socket_addrs()?.next().ok_or(format_err!("couldn't resolve to an address"))?;

    let socket = UdpSocket::bind("[::]:0")?;
    let mut protocols = Vec::new();
    const PROTO: &[u8] = b"hq-11";
    protocols.push(PROTO.len() as u8);
    protocols.extend_from_slice(PROTO);
    let config = quicr::Config {
        protocols,
        ..quicr::Config::default()
    };

    let reactor = tokio::reactor::Reactor::new()?;
    let timer = tokio_timer::Timer::new(reactor);
    
    let (endpoint, driver, _) = quicr::Endpoint::from_std(&tokio::reactor::Handle::current(), timer.handle(), socket, log.clone(), config, None)?;
    let mut executor = CurrentThread::new_with_park(timer);
    let request = format!("GET {}\r\n", url.path());

    executor.spawn(driver.map_err(|e| eprintln!("IO error: {}", e)));
    executor.block_on(
        endpoint.connect(&remote, url.host_str().map(|x| x.as_bytes()))
            .map_err(|e| format_err!("failed to connect: {}", e))
            .and_then(|(conn, _)| {
                eprintln!("connected!");
                conn.open_bi().map_err(|e| format_err!("failed to open stream: {}", e))
            })
            .and_then(|(send, recv)| {
                eprintln!("opened a stream");
                tokio::io::write_all(send, request.as_bytes()).map_err(|e| format_err!("failed to send request: {}", e))
                    .map(move |(send, _)| (send, recv))
            })
            .and_then(|(send, recv)| tokio::io::shutdown(send).map_err(|e| format_err!("failed to shutdown stream: {}", e))
                      .map(move |_| recv))
            .and_then(|recv| recv.read_to_end(usize::max_value()).map_err(|e| format_err!("failed to read response: {}", e)))
            .map(|data| {
                io::stdout().write_all(&data).unwrap();
                io::stdout().flush().unwrap();
                eprintln!("done")
            })
    ).map_err(|e| e.into_inner().unwrap())?;

    Ok(())
}