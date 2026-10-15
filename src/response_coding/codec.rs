use std::io::{self, BufRead, Read, Write};

use bytes::{Buf, Bytes};
use zstd::stream::raw::Operation;

use super::UpstreamCoding;

pub(super) const PIECE_BYTES: usize = 16 * 1024;

pub(super) enum Step {
    Output(Bytes),
    Input,
    Again,
    Done,
}

#[derive(Default)]
struct Input {
    bytes: Bytes,
    eof: bool,
}

impl Read for Input {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let len = self.fill_buf()?.len().min(output.len());
        output[..len].copy_from_slice(&self.bytes[..len]);
        self.consume(len);
        Ok(len)
    }
}

impl BufRead for Input {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.bytes.is_empty() && !self.eof {
            Err(io::ErrorKind::WouldBlock.into())
        } else {
            Ok(&self.bytes)
        }
    }

    fn consume(&mut self, len: usize) {
        self.bytes.advance(len);
    }
}

enum DecodeKind {
    Gzip(Box<flate2::bufread::MultiGzDecoder<Input>>),
    Deflate(flate2::Decompress, Input, bool),
    Zstd(zstd::stream::raw::Decoder<'static>, Input, bool),
}

pub(super) struct Decoder(DecodeKind);

impl Decoder {
    pub(super) fn new(coding: UpstreamCoding) -> io::Result<Self> {
        Ok(Self(match coding {
            UpstreamCoding::Gzip => DecodeKind::Gzip(Box::new(
                flate2::bufread::MultiGzDecoder::new(Input::default()),
            )),
            UpstreamCoding::Deflate => {
                DecodeKind::Deflate(flate2::Decompress::new(true), Input::default(), false)
            }
            UpstreamCoding::Zstd => {
                DecodeKind::Zstd(zstd::stream::raw::Decoder::new()?, Input::default(), false)
            }
        }))
    }

    pub(super) fn feed(&mut self, bytes: Bytes, eof: bool) {
        let input = match &mut self.0 {
            DecodeKind::Gzip(decoder) => decoder.get_mut(),
            DecodeKind::Deflate(_, input, _) | DecodeKind::Zstd(_, input, _) => input,
        };
        input.bytes = bytes;
        input.eof = eof;
    }

    pub(super) fn step(&mut self) -> io::Result<Step> {
        let mut output = [0u8; PIECE_BYTES];
        let (read, written, finished, input) = match &mut self.0 {
            DecodeKind::Gzip(decoder) => {
                return match decoder.read(&mut output) {
                    Ok(0) => Ok(Step::Done),
                    Ok(len) => Ok(Step::Output(Bytes::copy_from_slice(&output[..len]))),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(Step::Input),
                    Err(error) => Err(error),
                };
            }
            DecodeKind::Deflate(decoder, input, finished) => {
                if *finished {
                    if !input.bytes.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "trailing deflate data",
                        ));
                    }
                    return Ok(if input.eof { Step::Done } else { Step::Input });
                }
                let before_in = decoder.total_in();
                let before_out = decoder.total_out();
                let status = decoder
                    .decompress(&input.bytes, &mut output, flate2::FlushDecompress::None)
                    .map_err(io::Error::other)?;
                *finished = status == flate2::Status::StreamEnd;
                (
                    (decoder.total_in() - before_in) as usize,
                    (decoder.total_out() - before_out) as usize,
                    *finished,
                    input,
                )
            }
            DecodeKind::Zstd(decoder, input, finished) => {
                if *finished && input.bytes.is_empty() {
                    return Ok(if input.eof { Step::Done } else { Step::Input });
                }
                let status = decoder.run_on_buffers(&input.bytes, &mut output)?;
                *finished = status.remaining == 0;
                (status.bytes_read, status.bytes_written, *finished, input)
            }
        };
        input.bytes.advance(read);
        if written > 0 {
            return Ok(Step::Output(Bytes::copy_from_slice(&output[..written])));
        }
        if read > 0 || finished {
            return Ok(Step::Again);
        }
        if input.eof {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated compressed response",
            ));
        }
        if !input.bytes.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response decoder made no progress",
            ));
        }
        Ok(Step::Input)
    }
}

pub(super) enum Encoder {
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
    Deflate(flate2::write::ZlibEncoder<Vec<u8>>),
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
}

impl Encoder {
    pub(super) fn new(coding: UpstreamCoding) -> io::Result<Self> {
        Ok(match coding {
            UpstreamCoding::Gzip => Self::Gzip(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::default(),
            )),
            UpstreamCoding::Deflate => Self::Deflate(flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::default(),
            )),
            UpstreamCoding::Zstd => Self::Zstd(zstd::stream::write::Encoder::new(Vec::new(), 3)?),
        })
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> io::Result<Bytes> {
        match self {
            Self::Gzip(encoder) => {
                encoder.write_all(bytes)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()).into())
            }
            Self::Deflate(encoder) => {
                encoder.write_all(bytes)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()).into())
            }
            Self::Zstd(encoder) => {
                encoder.write_all(bytes)?;
                encoder.flush()?;
                Ok(std::mem::take(encoder.get_mut()).into())
            }
        }
    }

    pub(super) fn finish(self) -> io::Result<Bytes> {
        match self {
            Self::Gzip(encoder) => encoder.finish().map(Bytes::from),
            Self::Deflate(encoder) => encoder.finish().map(Bytes::from),
            Self::Zstd(encoder) => encoder.finish().map(Bytes::from),
        }
    }
}
