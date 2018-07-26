#![feature(transpose_result)]
#![feature(try_trait)]
#![deny(warnings)]

extern crate futures;
extern crate bytes;

use tokio_codec::Decoder;
use bytes::BytesMut;

#[macro_use]
extern crate enum_primitive;
extern crate num;

use enum_primitive::FromPrimitive;

use std::convert::From;
use bytes::Bytes;
use bytes::BufMut;
use tokio_codec::Encoder;


struct CachedQuery {
    header: Response,
    data: Bytes,
}

impl CachedQuery {
    pub fn new() -> CachedQuery {
        CachedQuery {
            header: Response::Invalid,
            data: Bytes::with_capacity(0),
        }
    }
}

enum_from_primitive! {
    enum Response {
        Legacy = 'm' as isize,
        New = 'I' as isize,
        Players = 'D' as isize,
        Invalid,
    }
}

impl From<Response> for u8 {
    fn from(r: Response) -> Self {
        match r {
            Response::Invalid => unreachable!(),
            any => any as u8,
        }
    }
}

// Decode response from bytes.
impl Decoder for CachedQuery {
    type Item = Self;
    type Error = Error;
    fn decode(&mut self, buf: &mut BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        if buf.len() < 5 || !buf.starts_with(&[0xFF; 4]) {
            return Ok(None);
        }
        let header = Response::from_u8(buf[4]).ok_or(Error::new(ErrorKind::InvalidData, "Invalid data"))?;

        Ok(Some(CachedQuery {
            header,
            data: buf.split_to(4).freeze()
        }))
    }
}

// Send Decoded QueryCodec to dst.
impl Encoder for CachedQuery {
    type Item = Self;
    type Error = Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> std::result::Result<(), Self::Error> {
        dst.put_slice(&[0xFF; 4]);
        dst.put_u8(From::from(item.header));
        dst.put(item.data);
        Ok(())
    }
}

mod tests {

    #[test]
    fn test_encoder() {
    }
}