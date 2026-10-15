use std::io;

use bytes::{Buf, Bytes};
use flate2::{Crc, Decompress, FlushDecompress, Status};

use super::{Input, Step, PIECE_BYTES};

enum State {
    Fixed([u8; 10], usize),
    ExtraLength([u8; 2], usize),
    Extra(usize),
    Name,
    Comment,
    HeaderCrc([u8; 2], usize),
    Body,
    Trailer([u8; 8], usize),
    Between,
}

pub(super) struct GzipDecoder {
    pub(super) input: Input,
    state: State,
    flags: u8,
    header_crc: Crc,
    body_crc: Crc,
    inflate: Decompress,
}

impl GzipDecoder {
    pub(super) fn new() -> Self {
        Self {
            input: Input::default(),
            state: State::Fixed([0; 10], 0),
            flags: 0,
            header_crc: Crc::new(),
            body_crc: Crc::new(),
            inflate: Decompress::new(false),
        }
    }

    fn next_header_field(&mut self) {
        self.state = if self.flags & 4 != 0 {
            self.flags &= !4;
            State::ExtraLength([0; 2], 0)
        } else if self.flags & 8 != 0 {
            self.flags &= !8;
            State::Name
        } else if self.flags & 16 != 0 {
            self.flags &= !16;
            State::Comment
        } else if self.flags & 2 != 0 {
            self.flags &= !2;
            State::HeaderCrc([0; 2], 0)
        } else {
            State::Body
        };
    }

    pub(super) fn step(&mut self) -> io::Result<Step> {
        if matches!(self.state, State::Body) {
            return self.inflate_piece();
        }
        if matches!(self.state, State::Between) {
            if self.input.bytes.is_empty() {
                return Ok(if self.input.eof {
                    Step::Done
                } else {
                    Step::Input
                });
            }
            self.header_crc.reset();
            self.body_crc.reset();
            self.inflate.reset(false);
            self.state = State::Fixed([0; 10], 0);
        }
        if self.input.bytes.is_empty() {
            return need_input(self.input.eof);
        }
        match &mut self.state {
            State::Fixed(bytes, filled) => {
                copy_field(
                    &mut self.input.bytes,
                    bytes,
                    filled,
                    Some(&mut self.header_crc),
                );
                if *filled == bytes.len() {
                    if bytes[..3] != [0x1f, 0x8b, 8] || bytes[3] & 0xe0 != 0 {
                        return Err(invalid("invalid gzip header"));
                    }
                    self.flags = bytes[3];
                    self.next_header_field();
                }
            }
            State::ExtraLength(bytes, filled) => {
                copy_field(
                    &mut self.input.bytes,
                    bytes,
                    filled,
                    Some(&mut self.header_crc),
                );
                if *filled == bytes.len() {
                    self.state = State::Extra(u16::from_le_bytes(*bytes) as usize);
                }
            }
            State::Extra(left) => {
                let len = (*left).min(self.input.bytes.len());
                self.header_crc.update(&self.input.bytes[..len]);
                self.input.bytes.advance(len);
                *left -= len;
                if *left == 0 {
                    self.next_header_field();
                }
            }
            State::Name | State::Comment => {
                // Scan optional text without accumulating peer-controlled header fields.
                let end = self.input.bytes.iter().position(|byte| *byte == 0);
                let len = end.map_or(self.input.bytes.len(), |index| index + 1);
                self.header_crc.update(&self.input.bytes[..len]);
                self.input.bytes.advance(len);
                if end.is_some() {
                    self.next_header_field();
                }
            }
            State::HeaderCrc(bytes, filled) => {
                copy_field(&mut self.input.bytes, bytes, filled, None);
                if *filled == bytes.len() {
                    if u16::from_le_bytes(*bytes) != self.header_crc.sum() as u16 {
                        return Err(invalid("gzip header checksum mismatch"));
                    }
                    self.next_header_field();
                }
            }
            State::Trailer(bytes, filled) => {
                copy_field(&mut self.input.bytes, bytes, filled, None);
                if *filled == bytes.len() {
                    let crc = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    if crc != self.body_crc.sum() || size != self.body_crc.amount() {
                        return Err(invalid("gzip body checksum or size mismatch"));
                    }
                    self.state = State::Between;
                }
            }
            State::Body | State::Between => unreachable!("handled before header parsing"),
        }
        Ok(Step::Again)
    }

    fn inflate_piece(&mut self) -> io::Result<Step> {
        let mut output = [0; PIECE_BYTES];
        let before_in = self.inflate.total_in();
        let before_out = self.inflate.total_out();
        // Drive inflate with empty input too: a back-reference can retain output
        // after the compressed piece ends. Temporary exhaustion is not body EOF.
        let status = self
            .inflate
            .decompress(&self.input.bytes, &mut output, FlushDecompress::None)
            .map_err(io::Error::other)?;
        let read = (self.inflate.total_in() - before_in) as usize;
        let written = (self.inflate.total_out() - before_out) as usize;
        self.input.bytes.advance(read);
        if status == Status::StreamEnd {
            self.state = State::Trailer([0; 8], 0);
        }
        if written > 0 {
            self.body_crc.update(&output[..written]);
            return Ok(Step::Output(Bytes::copy_from_slice(&output[..written])));
        }
        if read > 0 || status == Status::StreamEnd {
            return Ok(Step::Again);
        }
        if !self.input.bytes.is_empty() {
            return Err(invalid("gzip decoder made no progress"));
        }
        need_input(self.input.eof)
    }
}

fn copy_field(input: &mut Bytes, field: &mut [u8], filled: &mut usize, crc: Option<&mut Crc>) {
    let len = input.len().min(field.len() - *filled);
    field[*filled..*filled + len].copy_from_slice(&input[..len]);
    if let Some(crc) = crc {
        crc.update(&input[..len]);
    }
    input.advance(len);
    *filled += len;
}

fn need_input(eof: bool) -> io::Result<Step> {
    if eof {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated gzip response",
        ))
    } else {
        Ok(Step::Input)
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
