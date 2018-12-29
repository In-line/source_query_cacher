/// Some utility extensions are implemented here for several types
extern crate std;
use std::result::Result;

pub trait ResultExt<T, E> {
    fn map_option<U, F: FnOnce(T) -> U>(self, f: F) -> Result<Option<U>, E>;
    fn map_option_or_some<U, F: FnOnce(T) -> U>(self, default: U, f: F) -> Result<Option<U>, E>;
}

impl<T, E> ResultExt<T, E> for Result<Option<T>, E> {
    /// Maps a `Result<Option<T>, E>` to `Result<Option<U>, E>`
    ///
    /// ```
    /// # use source_query_cacher::util::ResultExt;
    /// let a : Result<Option<i32>, ()> = Ok(Some(1));
    /// let a : Result<Option<String>, ()> = a.map_option(|v| v.to_string());
    /// ```
    ///
    fn map_option<U, F: FnOnce(T) -> U>(self, f: F) -> Result<Option<U>, E> {
        self.map(|c| c.map(f))
    }

    /// Maps a `Result<Option<T>, E>` to `Result<Option<U>, E>`.
    /// If `Option<T>` is `None` then default `Some(U)` is used.
    ///
    /// ```
    /// # use source_query_cacher::util::ResultExt;
    /// let a : Result<Option<i32>, ()> = Ok(None);
    /// let a : Result<Option<String>, ()> = a.map_option_or_some("Nothing".to_string(), |v| v.to_string());
    /// ```
    fn map_option_or_some<U, F: FnOnce(T) -> U>(self, default: U, f: F) -> Result<Option<U>, E> {
        self.map(|c| c.map_or(Some(default), |c| Some(f(c))))
    }
}

extern crate bytes;
use self::bytes::{BigEndian, ByteOrder, Bytes, BytesMut};

pub trait BytesExt {
    fn is_not_split_packet(&self) -> bool;
    fn as_i32(&self) -> Option<i32>;
    fn from_i32(i: i32) -> Self;
}

impl BytesExt for BytesMut {
    fn is_not_split_packet(&self) -> bool {
        self.len() >= 5 && self.starts_with(&[0xFF; 4])
    }

    fn as_i32(&self) -> Option<i32> {
        if self.len() != 4 {
            None
        } else {
            Some(BigEndian::read_i32(&self))
        }
    }

    fn from_i32(i: i32) -> Self {
        let mut arr = [0; 4];
        BigEndian::write_i32(&mut arr, i);
        let mut buf = BytesMut::with_capacity(4);
        buf.extend_from_slice(&arr);
        buf
    }
}

impl BytesExt for Bytes {
    fn is_not_split_packet(&self) -> bool {
        self.len() >= 5 && self.starts_with(&[0xFF; 4])
    }

    fn as_i32(&self) -> Option<i32> {
        if self.len() != 4 {
            None
        } else {
            Some(BigEndian::read_i32(&self))
        }
    }

    fn from_i32(i: i32) -> Self {
        BytesMut::from_i32(i).freeze()
    }
}

extern crate tokio;
use self::tokio::prelude::*;

pub enum Tripple<A, B, C> {
    A(A),
    B(B),
    C(C),
}

impl<A, B, C> Future for Tripple<A, B, C>
where
    A: Future,
    B: Future<Item = A::Item, Error = A::Error>,
    C: Future<Item = A::Item, Error = A::Error>,
{
    type Item = A::Item;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<A::Item, A::Error> {
        match *self {
            Tripple::A(ref mut a) => a.poll(),
            Tripple::B(ref mut b) => b.poll(),
            Tripple::C(ref mut c) => c.poll(),
        }
    }
}

pub enum Quadro<A, B, C, D> {
    A(A),
    B(B),
    C(C),
    D(D),
}

impl<A, B, C, D> Future for Quadro<A, B, C, D>
where
    A: Future,
    B: Future<Item = A::Item, Error = A::Error>,
    C: Future<Item = A::Item, Error = A::Error>,
    D: Future<Item = A::Item, Error = A::Error>,
{
    type Item = A::Item;
    type Error = A::Error;

    fn poll(&mut self) -> Poll<A::Item, A::Error> {
        match *self {
            Quadro::A(ref mut a) => a.poll(),
            Quadro::B(ref mut b) => b.poll(),
            Quadro::C(ref mut c) => c.poll(),
            Quadro::D(ref mut d) => d.poll(),
        }
    }
}

use self::std::net::SocketAddr;
use std::cmp::{Ord, Ordering, PartialOrd};

#[derive(PartialEq, Eq, Hash, Clone, Copy, Into, From)]
pub struct SocketAddrOrdered(SocketAddr);

impl PartialOrd for SocketAddrOrdered {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self.0.ip(), self.0.port()).partial_cmp(&(other.0.ip(), other.0.port()))
    }
}

impl Ord for SocketAddrOrdered {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0.ip(), self.0.port()).cmp(&(other.0.ip(), other.0.port()))
    }
}
