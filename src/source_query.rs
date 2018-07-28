#![allow(dead_code)]

extern crate bytes;
extern crate std;
extern crate tokio_codec;

use self::bytes::{BufMut, Bytes, BytesMut};
use self::tokio_codec::{Decoder, Encoder};
use enum_primitive::FromPrimitive;

use util::BytesExt;

use std::convert::From;
use std::io;

enum_from_primitive! {
    #[derive(Debug, PartialEq, Clone, Eq, Hash)]
    pub enum ResponseHeader {
        LegacyInfo = 'm' as isize,
        NewInfo = 'I' as isize,
        Players = 'D' as isize,
        PlayersChallenge = 'A' as isize,
    }
}

enum_from_primitive! {
    #[derive(Debug, PartialEq, Clone, Eq, Hash)]
    pub enum RequestHeader {
        Info = 'T' as isize,
        Players = 'U' as isize, // Can require challenge number
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct SourceQuery {
    pub header: Header,
    pub data: Bytes,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Header {
    Response(ResponseHeader),
    Request(RequestHeader),
}

impl From<Header> for u8 {
    fn from(r: Header) -> Self {
        match r {
            Header::Response(r) => r as u8,
            Header::Request(r) => r as u8,
        }
    }
}

impl FromPrimitive for Header {
    fn from_i64(n: i64) -> Option<Self> {
        Header::from_u64(n as u64)
    }

    fn from_u64(n: u64) -> Option<Self> {
        if let Some(a) = ResponseHeader::from_u64(n).map(Header::Response) {
            return Some(a);
        }

        if let Some(a) = RequestHeader::from_u64(n).map(Header::Request) {
            return Some(a);
        }

        None
    }
}

impl SourceQuery {
    pub fn with(header: Header, data: Bytes) -> Self {
        SourceQuery { header, data }
    }

    pub fn with_request(header: RequestHeader, data: Bytes) -> Self {
        SourceQuery {
            header: Header::Request(header),
            data,
        }
    }

    pub fn with_response(header: ResponseHeader, data: Bytes) -> Self {
        SourceQuery {
            header: Header::Response(header),
            data,
        }
    }
}

#[derive(Default)]
pub struct SourceQueryCodec;

impl SourceQueryCodec {
    pub fn new() -> SourceQueryCodec {
        SourceQueryCodec
    }
}

// Decode response from bytes.
impl Decoder for SourceQueryCodec {
    type Item = SourceQuery;
    type Error = io::Error;
    fn decode(
        &mut self,
        buf: &mut BytesMut,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        if !buf.is_not_split_packet() {
            return Ok(None);
        }

        let header = Header::from_u8(buf[4]).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid header {}", buf[4]),
            )
        })?;

        Ok(Some(SourceQuery {
            header,
            data: buf.split_off(5).freeze(),
        }))
    }
}

// Send Decoded QueryCodec to dst.
impl Encoder for SourceQueryCodec {
    type Item = SourceQuery;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> std::result::Result<(), Self::Error> {
        dst.put_slice(&[0xFF; 4]);
        dst.put_u8(From::from(item.header));
        dst.put(item.data);
        Ok(())
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_decoder() {
        let slice = [0xFF, 0xFF, 0xFF, 0xFF, ('D' as u8), 1, 2, 3];

        assert_eq!(
            SourceQueryCodec::default()
                .decode(&mut BytesMut::from(&slice[..]))
                .unwrap()
                .unwrap(),
            SourceQuery::with(
                Header::Response(ResponseHeader::Players),
                Bytes::from(&slice[5..])
            )
        );

        assert_eq!(
            SourceQueryCodec::default()
                .decode(&mut BytesMut::from("Some random data"))
                .unwrap(),
            None
        );

        let slice = [0xFF, 0xFF, 0xFF, 0xFF, ('Z' as u8), 1, 2, 3];
        assert!(
            SourceQueryCodec::default()
                .decode(&mut BytesMut::from(&slice[..]))
                .is_err()
        );
    }

    #[test]
    fn test_encoder() {
        let slice = [0xFF, 0xFF, 0xFF, 0xFF, ('D' as u8), 1, 2, 3];

        let mut dst = BytesMut::new();

        assert!(
            SourceQueryCodec::default()
                .encode(
                    SourceQuery::with(
                        Header::Response(ResponseHeader::Players),
                        Bytes::from(&slice[5..])
                    ),
                    &mut dst
                ).is_ok()
        );
        assert_eq!(&slice[..], dst.as_ref());
    }
}
