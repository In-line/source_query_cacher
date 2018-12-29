//This file is part of source_query_cacher.
//
//source_query_cacher is free software: you can redistribute it and/or modify
//it under the terms of the GNU General Public License as published by
//the Free Software Foundation, either version 3 of the License, or
//(at your option) any later version.
//
//source_query_cacher is distributed in the hope that it will be useful,
//but WITHOUT ANY WARRANTY; without even the implied warranty of
//MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//GNU General Public License for more details.
//
//You should have received a copy of the GNU General Public License
//along with source_query_cacher.  If not, see <https://www.gnu.org/licenses/>.

extern crate futures;
extern crate itertools;
extern crate pretty_env_logger;
extern crate source_query_cacher;
extern crate structopt;
extern crate tokio;

use source_query_cacher::cacher;
use std::net::SocketAddr;
use std::time::Duration;
use structopt::StructOpt;

use itertools::*;
use tokio::prelude::future::*;
use tokio::prelude::stream::*;

#[derive(Debug, StructOpt, Clone)]
struct ServerClientPair {
    /// Server IP:PORT
    proxy: SocketAddr,
    /// Client IP:PORT
    server: SocketAddr,
}

#[derive(StructOpt, Debug, Clone)]
struct Options {
    #[structopt(short = "p", long = "update-period", default_value = "1000")]
    /// Update period in milliseconds.
    update_period: u64,

    #[structopt(short = "c", long = "chunk-size", default_value = "5")]
    /// Number of servers to dispatched on the same thread.
    chunk_size: usize,

    #[structopt(long = "challenge-number-expire", default_value = "1000")]
    /// Challenge number expire time in milliseconds.
    challenge_number_expire: u64,

    #[structopt(long = "client-queue-expire", default_value = "1000")]
    /// Client queue expire time in milliseconds.
    client_queue_expire: u64,

    #[structopt(
        long = "ignore-unknown-challenge_numbers",
        parse(try_from_str),
        default_value = "false"
    )]
    /// Ignore unknown challenge numbers. Some monitorings violate protocol and don't request challenge numbers from server.
    ignore_unknown_challenge_numbers: bool,

    #[structopt(short = "l", long = "list", raw(required = "true", min_values = "1"))]
    /// List of strings specified in "PROXY_IP:PORT SERVER_IP:PORT" format
    list: Vec<ServerClientPair>,
}

#[derive(Debug)]
enum InvalidProxyServerPairParseError {
    ProxyAddr(String, String),
    ServerAddr(String, String),
    Format(String),
}

impl std::fmt::Display for InvalidProxyServerPairParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self {
            InvalidProxyServerPairParseError::ProxyAddr(full, proxy) =>
               write!(f, "Can't parse proxy addr. Tried to parse \"{}\" which is the first part of \"{}\"", proxy, full),
            InvalidProxyServerPairParseError::ServerAddr(full, server) =>
                write!(f, "Can't parse server addr. Tried to parse \"{}\" which is the second part of \"{}\"", server, full),
            InvalidProxyServerPairParseError::Format(full) =>
                write!(f, "Can't parse pair of addrs. Tried to parse \"{}\" which can't be split by space to two parts. Valid syntax is \"PROXY:PORT SERVER:PORT\" ", full),
        }
    }
}

impl std::error::Error for InvalidProxyServerPairParseError {}

impl std::str::FromStr for ServerClientPair {
    type Err = InvalidProxyServerPairParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let splitted: Vec<&str> = s.split(' ').collect();
        if splitted.len() != 2 {
            Err(InvalidProxyServerPairParseError::Format(s.into()))
        } else {
            Ok(ServerClientPair {
                proxy: splitted[0].parse().map_err(|_| {
                    InvalidProxyServerPairParseError::ProxyAddr(s.into(), splitted[0].into())
                })?,
                server: splitted[1].parse().map_err(|_| {
                    InvalidProxyServerPairParseError::ServerAddr(s.into(), splitted[1].into())
                })?,
            })
        }
    }
}

fn main() {
    pretty_env_logger::init();
    let options = Options::from_args();

    let update_period = options.update_period;
    let challenge_number_expire = options.challenge_number_expire;
    let client_queue_expire = options.client_queue_expire;
    let ignore_unknown_challenge_numbers = options.ignore_unknown_challenge_numbers;

    tokio::run(
        iter_ok::<_, ()>(
            options
                .list
                .iter()
                .chunks(options.chunk_size)
                .into_iter()
                .map(|chunk| chunk.cloned().collect::<Vec<ServerClientPair>>())
                .collect::<Vec<_>>()
                .into_iter()
                .map(move |chunk| {
                    tokio::spawn(
                        join_all(chunk.into_iter().map(move |pair| {
                            cacher::cacher_run(
                                pair.proxy,
                                pair.server,
                                Duration::from_millis(update_period),
                                Duration::from_millis(challenge_number_expire),
                                Duration::from_millis(client_queue_expire),
                                ignore_unknown_challenge_numbers,
                            )
                            .or_else(|()| futures::future::ok::<(), std::io::Error>(()))
                        }))
                        .into_future()
                        .map(|_| {})
                        .map_err(|_| {}),
                    )
                }),
        )
        .for_each(|_| futures::future::ok::<(), ()>(())),
    );
}
