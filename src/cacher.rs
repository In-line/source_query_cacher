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

use self::futures::sync::mpsc::{unbounded, UnboundedSender};
use self::tokio::prelude::future::*;
use self::tokio::prelude::stream::*;
use self::tokio::prelude::*;

use super::frame::{Frame, FrameCodec};
use super::source_query::{Header, RequestHeader, ResponseHeader, SourceQuery};
use super::util::*;

use self::bytes::Bytes;
use self::fnv::FnvHashMap;
use self::lru_time_cache::*;
use self::rand::prelude::*;

use self::std::cell::RefCell;
use self::std::io;
use self::std::net::SocketAddr;
use self::std::time::{Duration, Instant};

use ref_thread_local::*;

use rand_core::block::BlockRng;

pub struct Cacher {
    addr: SocketAddr,
    server_addr: SocketAddr,
    cached_responses: FnvHashMap<ResponseHeader, bytes::Bytes>,
    cached_responses_up_to_date: FnvHashMap<RequestHeader, bool>,
    challenge_numbers: LruCache<SocketAddrOrdered, LruCache<i32, ()>>,
    clients_in_queue: FnvHashMap<ResponseHeader, LruCache<SocketAddrOrdered, ()>>,
    challenge_number_expire_duration: Duration,
    clock: Instant,
    update_period: Duration,
    client_queue_expire_duration: Duration,
    ignore_unknown_challenge_numbers: bool,
}

pub fn cacher_run(
    addr: SocketAddr,
    server_addr: SocketAddr,
    update_period: Duration,
    challenge_number_expire_duration: Duration,
    client_queue_expire_duration: Duration,
    ignore_unknown_challenge_numbers: bool,
) -> impl Future<Item = (), Error = ()> + Send {
    FutureRetry::new(
        move || {
            debug!(
                "Restarting run... Cacher addr: {} Server addr: {}",
                addr, server_addr
            );
            Cacher::new(
                addr,
                server_addr,
                update_period,
                challenge_number_expire_duration,
                client_queue_expire_duration,
                ignore_unknown_challenge_numbers,
            )
            .run()
        },
        |e| {
            debug!("Waiting 1 millis, because got an error {:?}", e);
            RetryPolicy::WaitRetry(Duration::from_millis(1))
        },
    )
}

macro_rules! add_client_to_queue {
    ($this:ident, $header:expr, $addr:expr) => {{
        $this
            .clients_in_queue
            .get_mut($header)
            .unwrap()
            .insert(Into::<SocketAddrOrdered>::into(*$addr), ());
    }};
}

impl Cacher {
    fn new(
        addr: SocketAddr,
        server_addr: SocketAddr,
        update_period: Duration,
        challenge_number_expire_duration: Duration,
        client_queue_expire_duration: Duration,
        ignore_unknown_challenge_numbers: bool,
    ) -> Self {
        let clients_in_queue = {
            let mut clients_in_queue = FnvHashMap::default();

            for header in &[
                ResponseHeader::LegacyInfo,
                ResponseHeader::NewInfo,
                ResponseHeader::Players,
            ] {
                clients_in_queue.insert(
                    *header,
                    LruCache::with_expiry_duration(client_queue_expire_duration),
                );
            }

            clients_in_queue
        };

        let cached_responses_up_to_date = {
            let mut cached_responses_up_to_date = FnvHashMap::default();

            for header in &[RequestHeader::Info, RequestHeader::Players] {
                cached_responses_up_to_date.insert(*header, false);
            }

            cached_responses_up_to_date
        };

        Cacher {
            addr,
            server_addr,
            cached_responses: FnvHashMap::default(),
            challenge_numbers: LruCache::with_expiry_duration(challenge_number_expire_duration),
            clients_in_queue,
            clock: Instant::now(),
            client_queue_expire_duration,
            update_period,
            cached_responses_up_to_date,
            ignore_unknown_challenge_numbers,
            challenge_number_expire_duration,
        }
    }

    #[inline]
    fn ignore() -> impl Future<Item = (), Error = io::Error> + Send {
        futures::future::ok(())
    }

    #[inline]
    fn send(
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        item: (SourceQuery, SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        futures::future::result(
            sender
                .clone()
                .unbounded_send(item)
                .map(|_| {})
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e)),
        )
    }

    fn exhaust_queue(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        header: ResponseHeader,
        data: &Bytes,
    ) -> impl Future<Item = (), Error = io::Error> {
        clone_all!(sender, header, data);

        trace!(
            "Exhausting queue: {:?} for {} clients",
            header,
            self.clients_in_queue.len()
        );

        join_all(
            self.clients_in_queue
                .insert(
                    header,
                    LruCache::with_expiry_duration(self.client_queue_expire_duration),
                )
                .unwrap()
                .iter()
                .map(|(key, _)| key)
                .cloned()
                .collect::<Vec<SocketAddrOrdered>>()
                .into_iter()
                .map(move |client| {
                    Cacher::send(
                        &sender.clone(),
                        (
                            SourceQuery::with(Header::Response(header), data.clone()),
                            client.into(),
                        ),
                    )
                }),
        )
        .map(|_| {})
    }

    fn process_response(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        (header, data, addr): (&ResponseHeader, &Bytes, &SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        match header {
            ResponseHeader::LegacyInfo | ResponseHeader::NewInfo | ResponseHeader::Players => {
                if *addr == self.server_addr {
                    debug!("Received {:?} from server.. So updating the cache.", header);

                    self.cached_responses.insert(*header, data.clone());
                    Tripple::A(self.exhaust_queue(&sender, *header, &data))
                } else {
                    trace!(
                        "Client {} is ignored because it sent {:?}, but it isn't server {}",
                        addr,
                        header,
                        self.server_addr
                    );
                    Tripple::B(Cacher::ignore())
                }
            }
            ResponseHeader::PlayersChallenge => {
                if let Some(i) = data.as_i32() {
                    if *addr != self.server_addr {
                        trace!(
                            "Client {} is ignored because it sent {:?}, but it isn't server {}",
                            addr,
                            header,
                            self.server_addr
                        );
                        Tripple::B(Cacher::ignore())
                    } else {
                        trace!("Using challenge id {} to request players from server", i);
                        Tripple::C(Cacher::send(
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
                    Tripple::B(Cacher::ignore())
                }
            }
        }
    }

    fn register_new_challenge_number(&mut self, addr: SocketAddr, new_challenge_number: i32) {
        let addr = Into::<SocketAddrOrdered>::into(addr);

        if !self.challenge_numbers.contains_key(&addr) {
            self.challenge_numbers.insert(
                addr,
                LruCache::with_expiry_duration(self.challenge_number_expire_duration),
            );
        }

        if let Some(challenge_numbers) = self.challenge_numbers.get_mut(&addr) {
            challenge_numbers.insert(new_challenge_number, ());
        } else {
            unreachable!()
        }
    }

    fn remove_challenge_number(&mut self, addr: SocketAddr, challenge_number: i32) -> Option<bool> {
        let addr = Into::<SocketAddrOrdered>::into(addr);
        if let Some(challenge_numbers) = self.challenge_numbers.get_mut(&addr) {
            let option = Some(challenge_numbers.remove(&challenge_number).is_some());

            if challenge_numbers.is_empty() {
                self.challenge_numbers.remove(&addr);
            }

            option
        } else {
            None
        }
    }

    fn process_players_request_with_challenge_number(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
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
                        ref_thread_local! {
                            static managed RNG: BlockRng<rand_hc::Hc128Core> = BlockRng::new(rand_hc::Hc128Core::from_entropy());
                        }
                        let challenge_number = RNG.borrow_mut().gen::<i32>();
                        if challenge_number == -1 {
                            0
                        } else {
                            challenge_number
                        }
                    };

                    self.register_new_challenge_number(addr.clone(), new_challenge_number);

                    return Tripple::A(Cacher::send(
                        sender,
                        (
                            SourceQuery::with_response(
                                ResponseHeader::PlayersChallenge,
                                Bytes::from_i32(new_challenge_number),
                            ),
                            *addr,
                        ),
                    ));
                } else if let Some(authorized) = self.remove_challenge_number(*addr, challenge) {
                    if authorized {
                        return Tripple::B(self.process_players_request(sender, addr));
                    } else {
                        trace!(
                            "Ignoring request with invalid challenge number {}",
                            challenge
                        );
                        self.challenge_numbers
                            .remove(&Into::<SocketAddrOrdered>::into(*addr));
                        return Tripple::C(Cacher::ignore());
                    }
                } else if !self.ignore_unknown_challenge_numbers {
                    trace!("Because ignoring unknown challenge numbers is inactive sending players to bastard {} with invalid challenge number: {}", addr, challenge);
                    return Tripple::B(self.process_players_request(sender, addr));
                }
            }
            debug!(
                    "Received invalid data for header {:?} from {}, so ignoring that bastard. This is data {:?} {}",
                    RequestHeader::Players,
                    addr,
                    data,
                    data.len()
                );
            Tripple::C(Cacher::ignore())
        }
    }

    fn process_players_request(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        addr: &SocketAddr,
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        if let Some(players) = self.cached_responses.get(&ResponseHeader::Players) {
            trace!(
                "Challenge number for a client {} is right. So sending him data!",
                addr
            );

            Either::A(
                Cacher::send(
                    sender,
                    (
                        SourceQuery::with_response(ResponseHeader::Players, players.clone()),
                        *addr,
                    ),
                )
                .join(self.request_update_if_needed(sender, addr, RequestHeader::Players))
                .map(|_| {}),
            )
        } else {
            Either::B(self.request_from_server(sender, addr, RequestHeader::Players))
        }
    }

    fn make_cached_responses_obsolete(&mut self) {
        if self.clock.elapsed() > self.update_period {
            self.clock = Instant::now();
            self.cached_responses_up_to_date
                .iter_mut()
                .for_each(|(_, i)| {
                    *i = false;
                });
            debug!(
                "Cache expired for {} after {:#?}. Seting flag for update...",
                self.server_addr, self.update_period
            )
        }
    }

    fn request_update_if_needed(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        addr: &SocketAddr,
        header: RequestHeader,
    ) -> impl Future<Item = (), Error = io::Error> + Send + 'static {
        if self
            .cached_responses_up_to_date
            .insert(header, true)
            .unwrap()
        {
            Either::A(Cacher::ignore())
        } else {
            Either::B(self.request_from_server(sender, addr, header))
        }
    }

    fn process_request(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        (header, data, addr): (&RequestHeader, &Bytes, &SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send + 'static {
        match header {
            RequestHeader::Info => {
                if *data != b"Source Engine Query\0"[..] {
                    trace!("Info header request with wrong signature was send from client {}. Ignoring that bastard. Data: {:?}", addr, data);
                    Quadro::D(Cacher::ignore())
                } else {
                    let legacy = self.cached_responses.get(&ResponseHeader::LegacyInfo);
                    let new = self.cached_responses.get(&ResponseHeader::NewInfo);

                    if legacy.is_none() && new.is_none() {
                        Quadro::A(self.request_from_server(sender, addr, RequestHeader::Info))
                    } else {
                        let mut to_send = Vec::with_capacity(2);

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

                        let sender_join = sender.clone();
                        let addr = *addr;
                        Quadro::B(
                            join_all(to_send.into_iter().filter_map(|i| i).map(
                                move |(header, data)| {
                                    Cacher::send(
                                        &sender_join.clone(),
                                        (SourceQuery::with_response(header, data), addr),
                                    )
                                },
                            ))
                            .join(self.request_update_if_needed(sender, &addr, RequestHeader::Info))
                            .map(|_| {}),
                        )
                    }
                }
            }
            RequestHeader::Players => {
                Quadro::C(self.process_players_request_with_challenge_number(sender, data, addr))
            }
        }
    }

    fn request_from_server(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        addr: &SocketAddr,
        header: RequestHeader,
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        match header {
            RequestHeader::Info => {
                trace!(
                    "Requesting cache for a client from server {}, because it is empty for players. Client {} will be in queue",
                    self.server_addr,
                    addr
                );
                add_client_to_queue!(self, &ResponseHeader::NewInfo, addr);
                add_client_to_queue!(self, &ResponseHeader::LegacyInfo, addr);
                Cacher::send(
                    sender,
                    (
                        SourceQuery::with_request(
                            RequestHeader::Info,
                            Bytes::from("Source Engine Query\0"),
                        ),
                        self.server_addr,
                    ),
                )
            }
            RequestHeader::Players => {
                trace!(
                    "Requesting cache for a client from server {}, because it is empty for {:?}. Client {} will be in queue",
                    self.server_addr,
                    RequestHeader::Players,
                    addr
                );

                add_client_to_queue!(self, &ResponseHeader::Players, addr);

                // Start to get cache from server
                Cacher::send(
                    sender,
                    (
                        SourceQuery::with_request(RequestHeader::Players, Bytes::from_i32(-1)),
                        self.server_addr,
                    ),
                )
            }
        }
    }

    fn process_query(
        &mut self,
        sender: &UnboundedSender<(SourceQuery, SocketAddr)>,
        (query, addr): &(SourceQuery, SocketAddr),
    ) -> impl Future<Item = (), Error = io::Error> + Send {
        match query.header {
            Header::Response(ref header) => {
                Either::A(self.process_response(sender, (header, &query.data, addr)))
            }
            Header::Request(ref header) => {
                self.make_cached_responses_obsolete();
                Either::B(self.process_request(sender, (header, &query.data, addr)))
            }
        }
    }

    fn run(self) -> Box<Future<Item = (), Error = ()> + Send + 'static> {
        let (sink, stream) =
            UdpFramed::new(UdpSocket::bind(&self.addr).unwrap(), FrameCodec::default()).split();

        let (sender_channel, sender_channel_receiver) = unbounded();
        let ref_self = RefCell::new(self);

        Box::new(
            stream
                .filter_map(|(frame, addr)| match frame {
                    Frame::SourceQuery(s) => Some((s, addr)),
                    Frame::None => None,
                })
                .retry(|e: io::Error| {
                    warn!("Error inside codec {:?}. Retrying..", e);
                    match e.kind() {
                        io::ErrorKind::Interrupted
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::Other
                        | io::ErrorKind::InvalidData
                        | io::ErrorKind::BrokenPipe => RetryPolicy::Repeat,
                        io::ErrorKind::PermissionDenied => RetryPolicy::ForwardError(e),
                        _ => RetryPolicy::WaitRetry(Duration::from_millis(1)),
                    }
                })
                .for_each(move |client| {
                    ref_self
                        .borrow_mut()
                        .process_query(&sender_channel, &client)
                })
                .map_err(|e| {
                    error!("Error inside process query. {:?}", e);
                })
                .join(
                    sender_channel_receiver
                        .map(|(query, addr)| (Frame::SourceQuery(query), addr))
                        .forward(sink.sink_map_err(|e| {
                            error!("Sink error inside receiver {}", e);
                        })),
                )
                .map(|_| {
                    info!(
                        "Stopped\
                         ."
                    );
                }),
        )
    }
}
