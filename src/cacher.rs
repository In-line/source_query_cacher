extern crate bytes;
extern crate fnv;
extern crate futures;
extern crate futures_retry;
extern crate lru_time_cache;
extern crate rand;
extern crate std;
extern crate tokio;

use self::futures_retry::{FutureRetry, RetryPolicy, StreamRetryExt};

use self::tokio::net::UdpFramed;
use self::tokio::net::UdpSocket;

use self::futures::sync::mpsc::{channel, Sender};
use self::tokio::prelude::future::*;
use self::tokio::prelude::stream::*;
use self::tokio::prelude::*;

use super::frame::{Frame, FrameCodec};
use super::source_query::{Header, RequestHeader, ResponseHeader, SourceQuery};
use super::util::*;

use self::bytes::Bytes;
use self::fnv::{FnvHashMap, FnvHashSet};
use self::lru_time_cache::*;
use self::rand::prelude::*;

use self::std::cell::RefCell;
use self::std::io;
use self::std::net::SocketAddr;
use self::std::time::{Duration, Instant};

pub struct Cacher {
    addr: SocketAddr,
    server_addr: SocketAddr,
    banned_clients: FnvHashSet<SocketAddr>,
    cached_responses: FnvHashMap<ResponseHeader, bytes::Bytes>,
    challenge_numbers: LruCache<SocketAddrOrdered, i32>,
    clients_in_queue: FnvHashMap<ResponseHeader, FnvHashSet<SocketAddr>>,
    clock: Instant,
    update_period: Duration,
}

pub fn cacher_run(
    addr: SocketAddr,
    server_addr: SocketAddr,
    update_period: Duration,
) -> Box<Future<Item = (), Error = ()> + Send + 'static> {
    Box::new(FutureRetry::new(
        move || {
            debug!("Restarting run...");
            Cacher::new(addr, server_addr, update_period).run()
        },
        |e| {
            debug!("Waiting 1 millis, because got an error {:?}", e);
            RetryPolicy::WaitRetry(Duration::from_millis(1))
        },
    ))
}

macro_rules! add_client_to_queue {
    ($this:ident, $header:expr, $addr:expr) => {{
        $this
            .clients_in_queue
            .get_mut($header)
            .unwrap()
            .insert(*$addr);
    }};
}

impl Cacher {
    fn new(addr: SocketAddr, server_addr: SocketAddr, update_period: Duration) -> Self {
        let clients_in_queue = {
            let mut clients_in_queue = FnvHashMap::default();

            for header in &[
                ResponseHeader::LegacyInfo,
                ResponseHeader::NewInfo,
                ResponseHeader::Players,
            ] {
                clients_in_queue.insert(header.clone(), FnvHashSet::default());
            }

            clients_in_queue
        };

        Cacher {
            addr,
            server_addr,
            banned_clients: FnvHashSet::default(),
            cached_responses: FnvHashMap::default(),
            challenge_numbers: LruCache::with_expiry_duration_and_capacity(
                Duration::from_secs(5),
                1000,
            ),
            clients_in_queue,
            clock: Instant::now(),
            update_period,
        }
    }

    fn gc(&mut self) {
        for (header, queue) in self.clients_in_queue.iter_mut() {
            if queue.len() > 1000 {
                info!("Clients queue for header {:?} seems to be full. Maybe server didn't respond? Removing {} clients. Server addr: {}", header, queue.len(), self.server_addr);
                queue.clear();
            }
        }
    }
    fn ignore_request() -> impl Future<Item = (), Error = io::Error> + Send {
        futures::future::ok(())
    }

    fn ban_client(&mut self, addr: SocketAddr) -> impl Future<Item = (), Error = io::Error> {
        self.banned_clients.insert(addr);
        Cacher::ignore_request()
    }

    fn send(
        sender: &Sender<(SourceQuery, SocketAddr)>,
        item: (SourceQuery, SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        sender
            .clone()
            .send(item)
            .map(|_| {})
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    fn exhaust_queue(
        &mut self,
        sender: &Sender<(SourceQuery, SocketAddr)>,
        header: &ResponseHeader,
        data: &Bytes,
    ) -> impl Future<Item = (), Error = io::Error> {
        let sender = sender.clone();
        let header = header.clone();
        let data = data.clone();

        join_all(
            self.clients_in_queue
                .insert(header.clone(), FnvHashSet::default())
                .unwrap()
                .into_iter()
                .map(move |client| {
                    Cacher::send(
                        &sender.clone(),
                        (
                            SourceQuery::with(Header::Response(header.clone()), data.clone()),
                            client,
                        ),
                    )
                }),
        ).map(|_| {})
    }

    fn process_response(
        &mut self,
        sender: &Sender<(SourceQuery, SocketAddr)>,
        (header, data, addr): (&ResponseHeader, &Bytes, &SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        match header {
            ResponseHeader::LegacyInfo | ResponseHeader::NewInfo | ResponseHeader::Players => {
                if *addr == self.server_addr {
                    debug!("Received {:?} from server.. So updating the cache.", header);

                    self.cached_responses.insert(header.clone(), data.clone());
                    Quadro::A(self.exhaust_queue(&sender, &header, &data))
                } else {
                    info!(
                        "Client {} is banned because it sent {:?}, but it isn't server {}",
                        addr, header, self.server_addr
                    );
                    Quadro::B(self.ban_client(*addr))
                }
            }
            ResponseHeader::PlayersChallenge => {
                if let Some(i) = data.as_i32() {
                    if *addr != self.server_addr {
                        info!(
                            "Client {} is banned because it sent {:?}, but it isn't server {}",
                            addr, header, self.server_addr
                        );
                        Quadro::B(self.ban_client(*addr))
                    } else {
                        trace!("Using challenge id {} to request players from server", i);
                        Quadro::C(Cacher::send(
                            sender,
                            (
                                SourceQuery::with_request(
                                    RequestHeader::Players,
                                    Bytes::from_i32(i),
                                ),
                                self.server_addr,
                            ),
                        ))
                    }
                } else {
                    trace!(
                        "Can't get challenge number from {:?}, so ignoring it from {}",
                        header,
                        addr
                    );
                    Quadro::D(Cacher::ignore_request())
                }
            }
        }
    }

    fn process_players_request(
        &mut self,
        sender: &Sender<(SourceQuery, SocketAddr)>,
        data: &Bytes,
        addr: &SocketAddr,
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        {
            if let Some(challenge) = data.as_i32() {
                if challenge == -1 {
                    trace!(
                        "Client {} requested new challenge number for. So sending him it!",
                        addr
                    );
                    // Client requested new challenge number
                    let new_challenge_number = {
                        let challenge_number = random::<i32>();
                        if challenge_number == -1 {
                            0
                        } else {
                            challenge_number
                        }
                    };

                    self.challenge_numbers
                        .insert(Into::<SocketAddrOrdered>::into(*addr), new_challenge_number);
                    return Tripple::B(Cacher::send(
                        sender,
                        (
                            SourceQuery::with_response(
                                ResponseHeader::PlayersChallenge,
                                Bytes::from_i32(new_challenge_number),
                            ),
                            *addr,
                        ),
                    ));
                } else if let Some(etalon_challenge) =
                    self.challenge_numbers
                        .remove(&Into::<SocketAddrOrdered>::into(*addr))
                {
                    if etalon_challenge == challenge {
                        if let Some(players) = self.cached_responses.get(&ResponseHeader::Players) {
                            trace!(
                                "Challenge number for a client {} is right. So sending him data!",
                                addr
                            );

                            return Tripple::B(Cacher::send(
                                sender,
                                (
                                    SourceQuery::with_response(
                                        ResponseHeader::Players,
                                        players.clone(),
                                    ),
                                    *addr,
                                ),
                            ));
                        } else {
                            trace!(
                                "Requesting cache for a client, because it is empty for {:?}. Client {} will not be in queue",
                                RequestHeader::Players,
                                addr
                            );

                            add_client_to_queue!(self, &ResponseHeader::Players, addr);

                            // Start to get cache from server
                            return Tripple::B(Cacher::send(
                                sender,
                                (
                                    SourceQuery::with_request(
                                        RequestHeader::Players,
                                        Bytes::from_i32(-1),
                                    ),
                                    self.server_addr,
                                ),
                            ));
                        }
                    } else {
                        trace!(
                            "Ignoring request with invalid challenge number {}",
                            challenge
                        );
                        return Tripple::C(Cacher::ignore_request());
                    }
                }
            }
            debug!(
                    "Received invalid data for header {:?} from {}, so banning that bastard. This is data {:?} {}",
                    RequestHeader::Players,
                    addr,
                    data,
                    data.len()
                );
            Tripple::A(self.ban_client(*addr))
        }
    }

    fn clear_cache_if_needed(&mut self) {
        if self.clock.elapsed() > self.update_period {
            self.clock = Instant::now();
            self.cached_responses.clear();
            debug!(
                "Cache expired for {} after {:#?}. Clearing...",
                self.server_addr, self.update_period
            )
        }
    }

    fn process_request(
        &mut self,
        sender: &Sender<(SourceQuery, SocketAddr)>,
        (header, data, addr): (&RequestHeader, &Bytes, &SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send + 'static {
        self.gc();

        match header {
            RequestHeader::Info => {
                let legacy = self.cached_responses.get(&ResponseHeader::LegacyInfo);
                let new = self.cached_responses.get(&ResponseHeader::NewInfo);

                if legacy.is_none() && new.is_none() {
                    trace!(
                            "Requesting cache for a client, because it is empty for {:?}. Client {} will be in queue",
                            header,
                            addr
                        );

                    add_client_to_queue!(self, &ResponseHeader::NewInfo, addr);
                    add_client_to_queue!(self, &ResponseHeader::LegacyInfo, addr);

                    Tripple::A(Cacher::send(
                        sender,
                        (
                            SourceQuery::with_request(
                                RequestHeader::Info,
                                Bytes::from("Source Engine Query"),
                            ),
                            self.server_addr,
                        ),
                    ))
                } else {
                    let mut to_send: Vec<Option<(ResponseHeader, Bytes)>> = Vec::with_capacity(2);

                    if let Some(legacy) = legacy {
                        trace!(
                            "Found {:?} info in cache, so sending it to {}",
                            header,
                            addr
                        );
                        to_send.push(Some((ResponseHeader::LegacyInfo, legacy.clone())));
                    }

                    if let Some(new) = new {
                        trace!("Found {:?} in cache, so sending it to {}", header, addr);
                        to_send.push(Some((ResponseHeader::NewInfo, new.clone())));
                    }

                    let sender = sender.clone();
                    let addr = *addr;
                    Tripple::B(
                        join_all(to_send.into_iter().filter_map(|i| i).map(
                            move |(header, data)| {
                                Cacher::send(
                                    &sender.clone(),
                                    (SourceQuery::with_response(header, data), addr),
                                )
                            },
                        )).map(|_| {}),
                    )
                }
            }
            RequestHeader::Players => Tripple::C(self.process_players_request(sender, data, addr)),
        }
    }

    fn process_query(
        &mut self,
        sender: &Sender<(SourceQuery, SocketAddr)>,
        (query, addr): &(SourceQuery, SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        if self.banned_clients.get(addr).is_none() {
            match query.header {
                Header::Response(ref header) => {
                    Tripple::A(self.process_response(sender, (header, &query.data, addr)))
                }
                Header::Request(ref header) => {
                    self.clear_cache_if_needed();
                    Tripple::B(self.process_request(sender, (header, &query.data, addr)))
                }
            }
        } else {
            Tripple::C(Cacher::ignore_request())
        }
    }

    fn run(self) -> Box<Future<Item = (), Error = ()> + Send + 'static> {
        let (sink, stream) =
            UdpFramed::new(UdpSocket::bind(&self.addr).unwrap(), FrameCodec::default()).split();

        let (sender_channel, sender_channel_receiver) = channel(50);
        let ref_self = RefCell::new(self);

        Box::new(
            stream
                .filter_map(|(frame, addr)| match frame {
                    Frame::SourceQuery(s) => Some((s, addr)),
                    Frame::None => None,
                }).retry(|e: io::Error| {
                    error!("Error inside codec {:?}. Retrying..", e);
                    match e.kind() {
                        io::ErrorKind::Interrupted
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::Other
                        | io::ErrorKind::BrokenPipe => RetryPolicy::Repeat,
                        io::ErrorKind::PermissionDenied => RetryPolicy::ForwardError(e),
                        _ => RetryPolicy::WaitRetry(Duration::from_millis(1)),
                    }
                }).for_each(move |client| {
                    ref_self
                        .borrow_mut()
                        .process_query(&sender_channel, &client)
                }).map_err(|e| {
                    error!("Error inside process query. {:?}", e);
                }).join(
                    sender_channel_receiver
                        .map(|(query, addr)| (Frame::SourceQuery(query), addr))
                        .forward(sink.sink_map_err(|e| {
                            error!("Sink error inside receiver {}", e);
                        })),
                ).map(|_| {
                    info!(
                        "Stopped\
                         ."
                    );
                }),
        )
    }
}
