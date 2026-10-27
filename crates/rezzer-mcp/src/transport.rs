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

/// How long `close` waits for error answers that are still being sent.
const CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The bytes read at once from the input.
const CHUNK: usize = 64 * 1024;

/// The longest `id` value that is kept to answer a line that is too long.
const MAX_ID_BYTES: usize = 256;

/// A line that was too long: the `id` of its request, when one was found.
struct TooLong {
    id: Option<RequestId>,
}

/// The server side of a line transport over `read` and `write`. A line over `max` bytes is
/// not read; it gets an error answer with the `id` of its request. When no `id` is found
/// (a notification, or an `id` the scan cannot read) the answer has no `id`, as rmcp writes
/// an error without one.
///
/// Memory: rmcp keeps the buffer of the longest line it has read for the rest of the
/// connection, so after one line near `max` the server holds about `max` bytes. While such a
/// message is read, the parsed `file` text and the decoded file come on top.
pub struct LineTransport<R: AsyncRead + Unpin, W: AsyncWrite> {
    inner: AsyncRwTransport<RoleServer, Guard<R>, W>,
    too_long: mpsc::UnboundedReceiver<TooLong>,
    max: usize,
    /// The send of the last error answer. Each answer runs as its own task after the one
    /// before, so rmcp dropping a `receive` future never drops or reorders an answer.
    answering: Option<tokio::task::JoinHandle<()>>,
}

impl<R, W> LineTransport<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(read: R, write: W, max: usize) -> Self {
        let (guard, too_long) = Guard::new(read, max);
        // The guard passes at most `max` bytes of a line and its line end, so rmcp drops
        // every line the guard reports, and reads every other line.
        let inner = AsyncRwTransport::new_server(guard, write).with_max_line_length(max);
        LineTransport {
            inner,
            too_long,
            max,
            answering: None,
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
                Some(line) = self.too_long.recv() => self.answer(line),
                message = self.inner.receive() => {
                    if message.is_none() {
                        // The input ended: lines reported just before the end still get
                        // their answer.
                        while let Ok(line) = self.too_long.try_recv() {
                            self.answer(line);
                        }
                    }
                    return message;
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        // The last answer task waits for the ones before it, so waiting for it is enough.
        if let Some(task) = self.answering.take() {
            match tokio::time::timeout(CLOSE_WAIT, task).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("rezzer: sending an error answer stopped: {e}"),
                Err(_) => eprintln!(
                    "rezzer: an error answer was not sent within {} seconds of the end",
                    CLOSE_WAIT.as_secs()
                ),
            }
        }
        self.inner.close().await
    }
}

impl<R, W> LineTransport<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    /// Sends the error answer for a line that was too long. The send runs as its own task,
    /// after the one before: rmcp drops a `receive` future when another branch of its loop
    /// is ready first, and an answer sent inside it would go with it.
    fn answer(&mut self, line: TooLong) {
        let answer = ServerJsonRpcMessage::error(
            ErrorData::invalid_request(too_long_message(self.max), None),
            line.id,
        );
        let send = self.inner.send(answer);
        let before = self.answering.take();
        self.answering = Some(tokio::spawn(async move {
            if let Some(before) = before
                && let Err(e) = before.await
            {
                eprintln!("rezzer: sending an error answer stopped: {e}");
            }
            if let Err(e) = send.await {
                eprintln!("rezzer: an error answer could not be sent: {e}");
            }
        }));
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
    /// The next line end in the chunk at or after `pos` (`filled` when there is none), so a
    /// chunk is searched once even when the reader takes it in small pieces.
    next_end: Option<usize>,
    max: usize,
    scan: IdScan,
    too_long: mpsc::UnboundedSender<TooLong>,
}

impl<R> Guard<R> {
    fn new(inner: R, max: usize) -> (Self, mpsc::UnboundedReceiver<TooLong>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let guard = Guard {
            inner,
            chunk: vec![0; CHUNK].into_boxed_slice(),
            pos: 0,
            filled: 0,
            line: 0,
            next_end: None,
            max,
            scan: IdScan::default(),
            too_long: tx,
        };
        (guard, rx)
    }
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
                this.next_end = None;
            }
            let before = out.filled().len();
            while this.pos < this.filled && out.remaining() > 0 {
                let next_end = match this.next_end {
                    Some(at) if at >= this.pos => at,
                    _ => {
                        let at = this.chunk[this.pos..this.filled]
                            .iter()
                            .position(|&b| b == b'\n')
                            .map_or(this.filled, |i| this.pos + i);
                        this.next_end = Some(at);
                        at
                    }
                };
                let rest = &this.chunk[this.pos..this.filled];
                let end = (next_end < this.filled).then_some(next_end - this.pos);
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
    use std::collections::BTreeSet;
    use std::time::Duration;

    use rmcp::model::JsonRpcMessage;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    use super::*;

    /// A reader that gives at most `step` bytes per read.
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        step: usize,
    }

    impl AsyncRead for Trickle {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            out: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            let n = this
                .step
                .min(out.remaining())
                .min(this.data.len() - this.pos);
            out.put_slice(&this.data[this.pos..this.pos + n]);
            this.pos += n;
            Poll::Ready(Ok(()))
        }
    }

    /// What the guard must give for `input`: every line as it is, but cut after `max` bytes
    /// with its line end kept, and a report for each line whose length with the line end is
    /// over `max`, as rmcp counts it.
    fn expected(input: &[u8], max: usize) -> (Vec<u8>, usize) {
        let mut out = Vec::new();
        let mut reports = 0;
        for line in input.split_inclusive(|&b| b == b'\n') {
            let (content, end) = match line.strip_suffix(b"\n") {
                Some(c) => (c, true),
                None => (line, false),
            };
            out.extend_from_slice(&content[..content.len().min(max)]);
            if end {
                out.push(b'\n');
                if line.len() > max {
                    reports += 1;
                }
            }
        }
        (out, reports)
    }

    /// The guard's output for `input`, read `step` bytes at a time from the source into a
    /// buffer of `room` bytes, and the ids it reported.
    async fn guarded(
        input: &[u8],
        max: usize,
        step: usize,
        room: usize,
    ) -> (Vec<u8>, Vec<Option<RequestId>>) {
        let source = Trickle {
            data: input.to_vec(),
            pos: 0,
            step,
        };
        let (mut guard, mut rx) = Guard::new(source, max);
        let mut out = Vec::new();
        let mut buf = vec![0; room];
        loop {
            let n = guard.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        let mut ids = Vec::new();
        while let Ok(line) = rx.try_recv() {
            ids.push(line.id);
        }
        (out, ids)
    }

    #[tokio::test]
    async fn the_guard_cuts_lines_as_rmcp_counts_them() {
        let max = 16;
        let mut input = Vec::new();
        for len in [max - 2, max - 1, max, max + 1, 40, 0, 3] {
            input.extend(std::iter::repeat_n(b'x', len));
            input.push(b'\n');
        }
        // CRLF: the carriage return counts, as in rmcp.
        input.extend_from_slice(&[b'y'; 14]);
        input.extend_from_slice(b"\r\n");
        input.extend_from_slice(&[b'y'; 15]);
        input.extend_from_slice(b"\r\n");
        // A last line without a line end is passed, cut, and not reported.
        input.extend_from_slice(&[b'z'; 30]);
        let (want, reports) = expected(&input, max);
        assert_eq!(reports, 4);
        for step in [1, 2, 7, 16, 17, CHUNK] {
            for room in [1, 2, 5, 15, 16, 17, 8192] {
                let (out, ids) = guarded(&input, max, step, room).await;
                assert_eq!(out, want, "step {step}, room {room}");
                assert_eq!(ids.len(), reports, "step {step}, room {room}");
            }
        }
    }

    #[tokio::test]
    async fn a_line_end_on_a_full_buffer_waits_for_the_next_read() {
        // The content fills the buffer exactly; the line end comes with the next read.
        let input = b"abcd\nefgh\n";
        let (out, ids) = guarded(input, 8, CHUNK, 4).await;
        assert_eq!(out, input);
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn the_id_after_the_cut_is_found() {
        let mut line =
            br#"{"jsonrpc":"2.0","method":"tools/call","params":{"id":1,"file":""#.to_vec();
        line.extend(std::iter::repeat_n(b'a', 500));
        line.extend_from_slice(br#""},"id":"late"}"#);
        line.push(b'\n');
        let (out, ids) = guarded(&line, 64, 3, 10).await;
        assert_eq!(out.len(), 65);
        assert_eq!(ids, [Some(RequestId::String("late".into()))]);
    }

    /// A ping request with `id`, padded with spaces to `len` bytes without its line end.
    fn ping(id: u32, len: usize) -> Vec<u8> {
        let head = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping""#);
        let mut line = head.into_bytes();
        assert!(line.len() < len);
        line.resize(len - 1, b' ');
        line.push(b'}');
        line
    }

    /// The ids of the error answers written to `out`, read until `n` came.
    async fn answers<R: AsyncRead + Unpin>(out: R, n: usize) -> BTreeSet<u64> {
        let mut lines = BufReader::new(out).lines();
        let mut ids = BTreeSet::new();
        while ids.len() < n {
            let line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
                .await
                .expect("an answer in time")
                .unwrap()
                .expect("a line");
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["error"]["code"], -32600, "{line}");
            ids.insert(v["id"].as_u64().expect("an id"));
        }
        ids
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_line_gets_exactly_one_answer_at_the_bound() {
        let max = 64;
        let mut input = Vec::new();
        let mut add = |id: u32, len: usize, crlf: bool| {
            input.extend(ping(id, len));
            input.extend_from_slice(if crlf { b"\r\n" } else { b"\n" });
        };
        add(1, max - 2, false); // max - 1 bytes with the line end
        add(2, max - 1, false); // max
        add(3, max, false); // max + 1
        add(4, 200, false);
        add(5, 40, false); // right after a line that was too long
        add(6, max - 2, true); // max with CRLF
        add(7, max - 1, true); // max + 1 with CRLF
        for step in [1, CHUNK] {
            let (server, client) = tokio::io::duplex(1 << 16);
            let (read, write) = tokio::io::split(server);
            let source = Trickle {
                data: input.clone(),
                pos: 0,
                step,
            };
            drop(read);
            let mut transport = LineTransport::new(source, write, max);
            let mut read_ids = BTreeSet::new();
            while let Some(message) = transport.receive().await {
                let JsonRpcMessage::Request(request) = message else {
                    panic!("a request");
                };
                let RequestId::Number(id) = request.id else {
                    panic!("a number id");
                };
                read_ids.insert(id as u64);
            }
            assert_eq!(read_ids, BTreeSet::from([1, 2, 5, 6]), "step {step}");
            // Closing waits for the answers still being sent.
            transport.close().await.unwrap();
            let (client_read, _client_write) = tokio::io::split(client);
            assert_eq!(
                answers(client_read, 3).await,
                BTreeSet::from([3, 4, 7]),
                "step {step}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_receive_keeps_the_answer() {
        // The answer cannot be written while the client does not read: the pipe holds 8 bytes.
        let (server, client) = tokio::io::duplex(8);
        let (read, write) = tokio::io::split(server);
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut transport = LineTransport::new(read, write, 64);
        let mut line = ping(9, 100);
        line.push(b'\n');
        let writer = tokio::spawn(async move {
            client_write.write_all(&line).await.unwrap();
            client_write
        });
        // rmcp drops the receive future when another branch of its loop is ready first.
        let dropped = tokio::time::timeout(Duration::from_millis(300), transport.receive()).await;
        assert!(dropped.is_err(), "no message is read");
        let _client_write = writer.await.unwrap();
        // The client reads only now; closing must still let the blocked answer through.
        let reader = tokio::spawn(answers(client_read, 1));
        transport.close().await.unwrap();
        assert_eq!(reader.await.unwrap(), BTreeSet::from([9]));
    }

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
