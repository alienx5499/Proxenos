//! Minimal `text/event-stream` frame parser (id, event, data; comments skipped).

use crate::Error;

/// A block larger than this without a frame boundary is a protocol error.
pub const MAX_FRAME: usize = 1 << 20;

/// One parsed block. `event` is empty for an id-only block: no dispatch, cursor update only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawFrame {
    pub id: Option<String>,
    pub event: String,
    pub data: String,
}

/// Feed it body chunks; it returns every frame completed so far.
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
    /// Bytes of `buf` already searched for a boundary.
    scanned: usize,
    started: bool,
}

impl Parser {
    /// Like `try_push`, but an oversized block is discarded instead of reported.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<RawFrame> {
        self.try_push(chunk).unwrap_or_default()
    }

    pub fn try_push(&mut self, chunk: &[u8]) -> Result<Vec<RawFrame>, Error> {
        let mut chunk = chunk;
        if !self.started && !chunk.is_empty() {
            self.started = true;
            chunk = chunk.strip_prefix("\u{feff}".as_bytes()).unwrap_or(chunk);
        }
        // CR is dropped outright, so CRLF works and lone-CR line endings do not.
        self.buf.extend(chunk.iter().filter(|b| **b != b'\r'));
        let mut out = Vec::new();
        loop {
            let from = self.scanned.saturating_sub(1);
            let Some(pos) = self.buf[from..].windows(2).position(|w| w == b"\n\n") else {
                break;
            };
            let end = from + pos + 2;
            let block: Vec<u8> = self.buf.drain(..end).collect();
            self.scanned = 0;
            if let Some(f) = parse_block(&String::from_utf8_lossy(&block)) {
                out.push(f);
            }
        }
        self.scanned = self.buf.len();
        if self.buf.len() > MAX_FRAME {
            self.buf.clear();
            self.scanned = 0;
            return Err(Error::Protocol(format!(
                "event frame over {MAX_FRAME} bytes"
            )));
        }
        Ok(out)
    }
}

fn parse_block(block: &str) -> Option<RawFrame> {
    let mut frame = RawFrame::default();
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "id" => frame.id = Some(value.to_owned()),
            "event" => frame.event = value.to_owned(),
            "data" => data.push(value),
            _ => {}
        }
    }
    if data.is_empty() {
        // Per the SSE spec a block without data is not dispatched; its id still counts.
        return frame.id.map(|id| RawFrame {
            id: Some(id),
            ..RawFrame::default()
        });
    }
    if frame.event.is_empty() {
        frame.event = "message".to_owned();
    }
    frame.data = data.join("\n");
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_split_across_chunks_and_keepalives() {
        let mut p = Parser::default();
        assert!(p.push(b": keepalive\n\nid: 7\nevent: mess").is_empty());
        let frames = p.push(b"age\r\ndata: {\"a\":1}\r\n\r\nevent: gap\ndata: {}\n\n");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].id.as_deref(), Some("7"));
        assert_eq!(frames[0].event, "message");
        assert_eq!(frames[0].data, "{\"a\":1}");
        assert_eq!(frames[1].event, "gap");
        assert_eq!(frames[1].id, None);
    }

    #[test]
    fn multi_line_data_joins_and_utf8_split_survives() {
        let mut p = Parser::default();
        let text = "data: a\ndata: é\n\n".as_bytes();
        let (a, b) = text.split_at(15); // inside the two-byte é
        assert!(p.push(a).is_empty());
        let f = p.push(b);
        assert_eq!(f[0].data, "a\né");
    }

    #[test]
    fn boundary_split_between_chunks_is_found() {
        let mut p = Parser::default();
        assert!(p.push(b"data: x\n").is_empty());
        assert_eq!(p.push(b"\n")[0].data, "x");
    }

    #[test]
    fn no_data_means_no_dispatch_but_id_counts_and_bom_is_stripped() {
        let mut p = Parser::default();
        let f = p.push("\u{feff}event: gap\n\nid: 9\n\ndata: y\n\n".as_bytes());
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].id.as_deref(), f[0].event.as_str()), (Some("9"), ""));
        assert_eq!(f[1].data, "y");
    }

    #[test]
    fn oversized_block_is_an_error() {
        let mut p = Parser::default();
        let big = vec![b'a'; MAX_FRAME + 1];
        assert!(matches!(p.try_push(&big), Err(Error::Protocol(_))));
        assert_eq!(p.push(b"data: ok\n\n")[0].data, "ok");
    }
}
