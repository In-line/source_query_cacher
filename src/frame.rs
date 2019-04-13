#![allow(dead_code)]

extern crate bytes;
extern crate std;
extern crate tokio_codec;

use crate::source_query::{SourceQuery, SourceQueryCodec};
use std::io;

use self::bytes::BytesMut;
use self::tokio_codec::{Decoder, Encoder};

use crate::util::ResultExt;

#[derive(Debug)]
pub enum Frame {
    SourceQuery(SourceQuery),
    None,
}

#[derive(Debug, Default)]
pub struct FrameCodec;

/// Bypasses some inconveniences while working with UDP.
impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = io::Error;
    fn decode(
        &mut self,
        buf: &mut BytesMut,
    ) -> std::result::Result<Option<Self::Item>, Self::Error> {
        SourceQueryCodec::default()
            .decode(buf)
            .map_option_or_some(Frame::None, Frame::SourceQuery)
    }
}

impl Encoder for FrameCodec {
    type Item = Frame;
    type Error = io::Error;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut BytesMut,
    ) -> std::result::Result<(), Self::Error> {
        match item {
            Frame::SourceQuery(c) => SourceQueryCodec::default().encode(c, dst),
            Frame::None => Err(io::Error::new(io::ErrorKind::Other, "Can't send nothing")),
        }
    }
}
