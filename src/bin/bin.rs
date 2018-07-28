extern crate env_logger;
extern crate futures;
extern crate source_query_cacher;
extern crate tokio;
#[macro_use]
extern crate structopt;

use source_query_cacher::cacher;
use std::net::SocketAddr;
use std::time::Duration;
use structopt::StructOpt;

use tokio::prelude::future::*;

#[derive(Debug, StructOpt)]
struct ServerClientPair {
    /// Server IP:PORT
    proxy: SocketAddr,
    /// Client IP:PORT
    server: SocketAddr,
}

#[derive(StructOpt, Debug)]
struct Options {
    #[structopt(short = "p", long = "update-period", default_value = "1000")]
    /// Update period in milliseconds.
    update_period: u64,
    #[structopt(short = "c", long = "chunk-size", default_value = "5")]
    /// Number of servers to dispatched on the same thread.
    chunks: usize,
    #[structopt(short = "l", long = "list", raw(required = "true", min_values = "1"))]
    /// List of strings specified in "PROXY_IP:PORT SERVER_IP:PORT" format
    list: Vec<ServerClientPair>,
}

#[derive(Debug)]
enum ProxyServerPairParseError {
    InvalidProxyAddr(String, String),
    InvalidServerAddr(String, String),
    InvalidFormat(String),
}

impl std::fmt::Display for ProxyServerPairParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self {
            ProxyServerPairParseError::InvalidProxyAddr(full, proxy) =>
               write!(f, "Can't parse proxy addr. Tried to parse \"{}\" which is the first part of \"{}\"", proxy, full),
            ProxyServerPairParseError::InvalidServerAddr(full, server) =>
                write!(f, "Can't parse server addr. Tried to parse \"{}\" which is the second part of \"{}\"", server, full),
            ProxyServerPairParseError::InvalidFormat(full) =>
                write!(f, "Can't parse pair of addrs. Tried to parse \"{}\" which can't be split by space to two parts. Valid syntax is \"PROXY:PORT SERVER:PORT\" ", full),
        }
    }
}

impl std::error::Error for ProxyServerPairParseError {}

impl std::str::FromStr for ServerClientPair {
    type Err = ProxyServerPairParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split(' ').collect();
        if splitted.len() != 2 {
            Err(ProxyServerPairParseError::InvalidFormat(s.into()))
        } else {
            Ok(ServerClientPair {
                proxy: splitted[0].parse().map_err(|_| {
                    ProxyServerPairParseError::InvalidProxyAddr(s.into(), splitted[0].into())
                })?,
                server: splitted[1].parse().map_err(|_| {
                    ProxyServerPairParseError::InvalidServerAddr(s.into(), splitted[1].into())
                })?,
            })
        }
    }
}

fn main() {
    env_logger::init();

    let options = Options::from_args();
    let period = options.update_period;
    tokio::run(
        join_all(options.list.into_iter().map(move |pair| {
            cacher::cacher_run(pair.proxy, pair.server, Duration::from_millis(period))
                .or_else(|()| futures::future::ok::<(), std::io::Error>(()))
        })).map(|_| {})
        .map_err(|_| {}),
    );
}
