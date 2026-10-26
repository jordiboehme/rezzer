// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The stdio transport: one JSON-RPC message per line, with a bound on the size of a line.
//!
//! rmcp's line transport drops a line over its bound and only logs it, so the client would
//! wait for an answer that never comes. Rezzer's bound follows the file size it advertises,
//! and a line over it gets an error answer.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use rmcp::RoleServer;
use rmcp::model::{ErrorData, RequestId, ServerJsonRpcMessage};
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::transport::async_rw::AsyncRwTransport;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

/// Room in a message for everything next to the file: the JSON-RPC envelope, the tool name
/// and the other arguments.
const ENVELOPE_BYTES: u64 = 1 << 20;

/// The longest line read, in bytes with its line end, for inline files of up to
/// `max_inline_bytes`. A file of that size fits in every JSON form of a `data:` URI: base64
/// (4 bytes per 3), percent-encoded (3 bytes per byte) and as JSON `\u00XX` escapes (6 bytes
/// per byte).
pub fn max_message_bytes(max_inline_bytes: u64) -> usize {
    let bytes = max_inline_bytes
        .saturating_mul(6)
        .saturating_add(ENVELOPE_BYTES);
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

/// The bytes read at once from the input.
const CHUNK: usize = 64 * 1024;

/// The longest `id` value that is kept to answer a line that is too long.
const MAX_ID_BYTES: usize = 256;

/// A line that was too long: the `id` of its request, when one was found.
struct TooLong {
    id: Option<RequestId>,
}

/// The server side of a line transport over `read` and `write`. A line over `max` bytes is
/// not read; it gets an error answer with the `id` of its request, or a null `id` when none
/// is found.
pub struct LineTransport<R: AsyncRead + Unpin, W: AsyncWrite> {
    inner: AsyncRwTransport<RoleServer, Guard<R>, W>,
    too_long: mpsc::UnboundedReceiver<TooLong>,
    max: usize,
}

impl<R, W> LineTransport<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(read: R, write: W, max: usize) -> Self {
        let (tx, too_long) = mpsc::unbounded_channel();
        let guard = Guard {
            inner: read,
            chunk: vec![0; CHUNK].into_boxed_slice(),
            pos: 0,
            filled: 0,
            line: 0,
            max,
            scan: IdScan::default(),
            too_long: tx,
        };
        // The guard passes at most `max` bytes of a line and its line end, so rmcp drops
        // every line the guard reports, and reads every other line.
        let inner = AsyncRwTransport::new_server(guard, write).with_max_line_length(max);
        LineTransport {
            inner,
            too_long,
            max,
        }
    }
}

impl<R, W> Transport<RoleServer> for LineTransport<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        loop {
            tokio::select! {
                biased;
                Some(line) = self.too_long.recv() => {
                    let answer = ServerJsonRpcMessage::error(
                        ErrorData::invalid_request(too_long_message(self.max), None),
                        line.id,
                    );
                    if self.inner.send(answer).await.is_err() {
                        return None;
                    }
                }
                message = self.inner.receive() => return message,
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}

fn too_long_message(max: usize) -> String {
    format!(
        "the message is longer than the {max} bytes this server reads in one message; \
         give a larger file by path"
    )
}

/// Passes the input on and cuts every line after `max` bytes. It keeps the line end, so the
/// reader behind it sees a line of `max + 1` bytes and drops it. The `id` of such a line is
/// sent to the transport when its line end comes.
struct Guard<R> {
    inner: R,
    chunk: Box<[u8]>,
    pos: usize,
    filled: usize,
    /// Bytes of the current line so far, its line end included.
    line: usize,
    max: usize,
    scan: IdScan,
    too_long: mpsc::UnboundedSender<TooLong>,
}

impl<R: AsyncRead + Unpin> AsyncRead for Guard<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if out.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if this.pos == this.filled {
                let mut buf = ReadBuf::new(&mut this.chunk);
                ready!(Pin::new(&mut this.inner).poll_read(cx, &mut buf))?;
                let n = buf.filled().len();
                if n == 0 {
                    return Poll::Ready(Ok(()));
                }
                this.pos = 0;
                this.filled = n;
            }
            let before = out.filled().len();
            while this.pos < this.filled && out.remaining() > 0 {
                let rest = &this.chunk[this.pos..this.filled];
                let end = rest.iter().position(|&b| b == b'\n');
                let run = end.unwrap_or(rest.len());
                let room = this.max.saturating_sub(this.line);
                let pass = run.min(room).min(out.remaining());
                let take = if pass < run.min(out.remaining()) {
                    // Over the bound: the rest of the run is read and dropped.
                    run
                } else {
                    pass
                };
                this.scan.feed(&rest[..take]);
                out.put_slice(&rest[..pass]);
                this.pos += take;
                this.line = this.line.saturating_add(take);
                if take == run && end.is_some() && out.remaining() > 0 {
                    this.pos += 1;
                    this.line = this.line.saturating_add(1);
                    out.put_slice(b"\n");
                    if this.line > this.max {
                        // The transport may be gone already; then nobody waits for an answer.
                        let _ = this.too_long.send(TooLong { id: this.scan.id() });
                    }
                    this.line = 0;
                    this.scan = IdScan::default();
                }
            }
            if out.filled().len() > before {
                return Poll::Ready(Ok(()));
            }
        }
    }
}

/// Finds the value of the top-level `id` key of a JSON object while its bytes go by, in any
/// position of the object and without keeping the rest.
#[derive(Default)]
struct IdScan {
    /// Open objects and arrays.
    depth: u32,
    in_string: bool,
    escape: bool,
    /// The next string at depth 1 is a key.
    want_key: bool,
    /// The bytes of the key being read, while it may still be `id`.
    key: Option<Vec<u8>>,
    /// The last key at depth 1 was `id` and its value has not started.
    after_id_key: bool,
    /// The bytes of the `id` value while it is read; `None` once it grew too long.
    value: Option<Vec<u8>>,
    reading_value: bool,
    found: Option<Vec<u8>>,
    broken: bool,
}

impl IdScan {
    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        if self.reading_value {
            let ends = !self.in_string && self.depth == 1 && (b == b',' || b == b'}');
            if ends {
                self.reading_value = false;
                self.found = self.value.take();
            } else if let Some(v) = &mut self.value {
                if v.len() < MAX_ID_BYTES {
                    v.push(b);
                } else {
                    self.value = None;
                }
            }
        }
        if self.in_string {
            if self.escape {
                self.escape = false;
            } else if b == b'\\' {
                self.escape = true;
                self.key = None;
            } else if b == b'"' {
                self.in_string = false;
                if let Some(key) = self.key.take() {
                    self.after_id_key = key == b"id";
                }
            } else if let Some(key) = &mut self.key {
                if key.len() < 2 {
                    key.push(b);
                } else {
                    self.key = None;
                }
            }
            return;
        }
        match b {
            b'"' => {
                self.in_string = true;
                if self.depth == 1 && self.want_key {
                    self.want_key = false;
                    self.key = Some(Vec::new());
                }
            }
            b'{' | b'[' => {
                self.depth = self.depth.saturating_add(1);
                if self.depth == 1 {
                    self.want_key = b == b'{';
                    self.broken |= b == b'[';
                }
            }
            b'}' | b']' => self.depth = self.depth.saturating_sub(1),
            b',' if self.depth == 1 => self.want_key = true,
            b':' if self.depth == 1 && self.after_id_key => {
                self.after_id_key = false;
                self.reading_value = true;
                self.value = Some(Vec::new());
            }
            _ => {}
        }
    }

    /// The `id` when it was found and is a number or a string.
    fn id(&self) -> Option<RequestId> {
        if self.broken {
            return None;
        }
        let raw = self.found.as_deref()?;
        serde_json::from_slice(raw.trim_ascii()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_of(text: &str) -> Option<RequestId> {
        let mut scan = IdScan::default();
        for chunk in text.as_bytes().chunks(3) {
            scan.feed(chunk);
        }
        scan.id()
    }

    #[test]
    fn finds_the_top_level_id_anywhere() {
        let first = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{}}"#;
        assert_eq!(id_of(first), Some(RequestId::Number(7)));
        let last = r#"{"params":{"id":1,"arguments":{"file":"data:,x\"}"}},"id":"a-1"}"#;
        assert_eq!(id_of(last), Some(RequestId::String("a-1".into())));
        let spaced = "{ \"id\" : 42 , \"method\" : \"x\" }";
        assert_eq!(id_of(spaced), Some(RequestId::Number(42)));
    }

    #[test]
    fn no_id_without_a_top_level_one() {
        assert_eq!(id_of(r#"{"params":{"id":3}}"#), None);
        assert_eq!(id_of(r#"[{"id":3}]"#), None);
        assert_eq!(id_of(r#"{"id":null}"#), None);
        assert_eq!(id_of(r#"{"id":{"x":1}}"#), None);
        assert_eq!(id_of(r#"{"idx":5}"#), None);
        assert_eq!(id_of("not json"), None);
    }

    #[test]
    fn the_bound_holds_for_large_files() {
        assert_eq!(max_message_bytes(0), 1 << 20);
        assert_eq!(max_message_bytes(10), 60 + (1 << 20));
        assert!(max_message_bytes(u64::MAX) >= usize::MAX / 2);
    }
}
