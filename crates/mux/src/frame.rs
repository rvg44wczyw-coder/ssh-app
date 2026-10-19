//! Wire framing between a mux client and the mux server: `type: u8`, `len: u32 big endian`,
//! payload. Length is capped, unknown types and oversized frames are errors (the connection is
//! dropped), and the decoder accepts input in arbitrary fragments.

pub const MAX_PAYLOAD: usize = 1 << 20;
const HEADER: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Client -> server: bytes typed by the user.
    Input(Vec<u8>),
    /// Server -> client: bytes for the terminal.
    Output(Vec<u8>),
    /// Client -> server: the viewport changed.
    Resize { cols: u16, rows: u16 },
    /// Client -> server: first frame, names the session and the protocol version.
    Hello { version: u8, session: String },
    /// Server -> client: the session ended with this exit code.
    Exit(i32),
    /// Client -> server: detach, leave the session running.
    Detach,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooLarge(usize),
    UnknownType(u8),
    Malformed(&'static str),
}

const T_INPUT: u8 = 1;
const T_OUTPUT: u8 = 2;
const T_RESIZE: u8 = 3;
const T_HELLO: u8 = 4;
const T_EXIT: u8 = 5;
const T_DETACH: u8 = 6;

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let (t, payload): (u8, Vec<u8>) = match self {
            Frame::Input(b) => (T_INPUT, b.clone()),
            Frame::Output(b) => (T_OUTPUT, b.clone()),
            Frame::Resize { cols, rows } => {
                let mut p = cols.to_be_bytes().to_vec();
                p.extend_from_slice(&rows.to_be_bytes());
                (T_RESIZE, p)
            }
            Frame::Hello { version, session } => {
                let mut p = vec![*version];
                p.extend_from_slice(session.as_bytes());
                (T_HELLO, p)
            }
            Frame::Exit(code) => (T_EXIT, code.to_be_bytes().to_vec()),
            Frame::Detach => (T_DETACH, Vec::new()),
        };
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.push(t);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    fn decode(t: u8, p: &[u8]) -> Result<Frame, FrameError> {
        match t {
            T_INPUT => Ok(Frame::Input(p.to_vec())),
            T_OUTPUT => Ok(Frame::Output(p.to_vec())),
            T_RESIZE => {
                let b: [u8; 4] = p.try_into().map_err(|_| FrameError::Malformed("resize"))?;
                let (cols, rows) = (
                    u16::from_be_bytes([b[0], b[1]]),
                    u16::from_be_bytes([b[2], b[3]]),
                );
                if cols == 0 || rows == 0 {
                    return Err(FrameError::Malformed("resize to zero"));
                }
                Ok(Frame::Resize { cols, rows })
            }
            T_HELLO => {
                let (v, name) = p.split_first().ok_or(FrameError::Malformed("hello"))?;
                let session = String::from_utf8(name.to_vec())
                    .map_err(|_| FrameError::Malformed("hello name"))?;
                Ok(Frame::Hello {
                    version: *v,
                    session,
                })
            }
            T_EXIT => {
                let b: [u8; 4] = p.try_into().map_err(|_| FrameError::Malformed("exit"))?;
                Ok(Frame::Exit(i32::from_be_bytes(b)))
            }
            T_DETACH if p.is_empty() => Ok(Frame::Detach),
            T_DETACH => Err(FrameError::Malformed("detach")),
            other => Err(FrameError::UnknownType(other)),
        }
    }
}

/// Collects bytes and yields whole frames. After an error the decoder must be discarded.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < HEADER {
            return Ok(None);
        }
        let len = u32::from_be_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if self.buf.len() < HEADER + len {
            return Ok(None);
        }
        let t = self.buf[0];
        let frame = Frame::decode(t, &self.buf[HEADER..HEADER + len])?;
        self.buf.drain(..HEADER + len);
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Frame> {
        vec![
            Frame::Hello {
                version: 1,
                session: "claude-1".into(),
            },
            Frame::Input(b"ls\r".to_vec()),
            Frame::Output(vec![0x1b, b'[', b'H', 0xff]),
            Frame::Resize { cols: 80, rows: 24 },
            Frame::Exit(-1),
            Frame::Detach,
            Frame::Input(Vec::new()),
        ]
    }

    #[test]
    fn round_trips_in_any_fragmentation() {
        let bytes: Vec<u8> = all().iter().flat_map(Frame::encode).collect();
        for step in [1, 2, 3, 7, bytes.len()] {
            let mut d = Decoder::default();
            let mut got = Vec::new();
            for chunk in bytes.chunks(step) {
                d.push(chunk);
                while let Some(f) = d.next_frame().unwrap() {
                    got.push(f);
                }
            }
            assert_eq!(got, all(), "step {step}");
        }
    }

    #[test]
    fn hostile_input_is_an_error_not_a_panic_or_an_allocation() {
        let mut d = Decoder::default();
        d.push(&[T_INPUT, 0xff, 0xff, 0xff, 0xff]);
        assert!(matches!(d.next_frame(), Err(FrameError::TooLarge(_))));
        for (t, p) in [
            (9u8, vec![]),
            (T_RESIZE, vec![0, 0, 0]),
            (T_RESIZE, vec![0, 0, 0, 5]),
            (T_HELLO, vec![]),
            (T_HELLO, vec![1, 0xff, 0xfe]),
            (T_EXIT, vec![1]),
            (T_DETACH, vec![1]),
        ] {
            let mut d = Decoder::default();
            let mut raw = vec![t];
            raw.extend_from_slice(&(p.len() as u32).to_be_bytes());
            raw.extend_from_slice(&p);
            d.push(&raw);
            assert!(d.next_frame().is_err(), "type {t} {p:?}");
        }
        // Incomplete frames just wait.
        let mut d = Decoder::default();
        d.push(&[T_INPUT, 0, 0, 0, 10, 1, 2]);
        assert_eq!(d.next_frame(), Ok(None));
    }
}
