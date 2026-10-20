//! A live VNC connection: a reader task decoding server messages into a
//! framebuffer and a writer task sending client messages.
//!
//! Everything sent to the UI goes through a [`Sink`] as binary messages whose
//! first byte is the kind (all integers little-endian):
//!
//! * `0` — JSON [`Event`] (UTF-8).
//! * `1` — changed pixels: `u32` count, then per rectangle `u16` x, y, w, h
//!   followed by `w*h*4` RGBA bytes.
//! * `2` — cursor shape: `u16` hot x, hot y, w, h, then `w*h*4` RGBA bytes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::rfb::{self, CursorShape, Decoder, Framebuffer, Rect, ServerInit, ServerMessage, proto};

pub const MSG_EVENT: u8 = 0;
pub const MSG_RECTS: u8 = 1;
pub const MSG_CURSOR: u8 = 2;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the UI to draw a frame before asking for the next one.
const ACK_TIMEOUT: Duration = Duration::from_millis(500);
/// Above this many rectangles, a single bounding box is sent instead.
const MAX_RECTS_PER_FRAME: usize = 64;

/// Receives everything the session produces for the UI.
pub trait Sink: Send + Sync + 'static {
    fn message(&self, bytes: Vec<u8>);
    fn cut_text(&self, text: String);
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Resize {
        width: u16,
        height: u16,
        name: String,
    },
    Bell,
    Disconnected {
        reason: Option<rfb::ErrorInfo>,
    },
}

/// Input sent by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputEvent {
    Pointer { x: u16, y: u16, mask: u8 },
    Key { keysym: u32, down: bool },
}

enum Outgoing {
    Bytes(Vec<u8>),
    FullRefresh,
}

pub struct Session {
    out: mpsc::UnboundedSender<Outgoing>,
    ack: Arc<Notify>,
    tasks: [JoinHandle<()>; 2],
}

impl Drop for Session {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl Session {
    /// Opens a TCP connection, performs the handshake and starts the session.
    pub async fn connect<S: Sink>(
        host: &str,
        port: u16,
        password: Option<&str>,
        shared: bool,
        sink: S,
    ) -> rfb::Result<Session> {
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
            .await
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))??;
        stream.set_nodelay(true)?;
        tokio::time::timeout(CONNECT_TIMEOUT, Self::start(stream, password, shared, sink))
            .await
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?
    }

    /// Performs the handshake on an established stream and starts the session.
    pub async fn start<T, S>(
        mut stream: T,
        password: Option<&str>,
        shared: bool,
        sink: S,
    ) -> rfb::Result<Session>
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
        S: Sink,
    {
        let init = rfb::handshake(&mut stream, password, shared).await?;
        log::info!(
            "connected to {:?} ({}×{}), server pixel format {:?}",
            init.name,
            init.width,
            init.height,
            init.pixel_format
        );

        let (rd, wr) = tokio::io::split(stream);
        let (out, rx) = mpsc::unbounded_channel();
        let ack = Arc::new(Notify::new());
        let size = Arc::new(AtomicU32::new(pack_size(init.width, init.height)));

        let _ = out.send(Outgoing::Bytes(proto::set_pixel_format(
            &proto::PixelFormat::RGBX,
        )));
        let _ = out.send(Outgoing::Bytes(proto::set_encodings(proto::ENCODINGS)));
        let _ = out.send(Outgoing::FullRefresh);

        let writer = tokio::spawn(writer_loop(wr, rx, size.clone()));
        let reader = tokio::spawn(reader_loop(
            BufReader::with_capacity(1 << 18, rd),
            init,
            sink,
            out.clone(),
            ack.clone(),
            size,
        ));
        Ok(Session {
            out,
            ack,
            tasks: [reader, writer],
        })
    }

    pub fn input(&self, events: &[InputEvent]) {
        for ev in events {
            let bytes = match *ev {
                InputEvent::Pointer { x, y, mask } => proto::pointer_event(mask, x, y),
                InputEvent::Key { keysym, down } => proto::key_event(down, keysym),
            };
            let _ = self.out.send(Outgoing::Bytes(bytes));
        }
    }

    pub fn send_cut_text(&self, text: &str) {
        let _ = self.out.send(Outgoing::Bytes(proto::client_cut_text(text)));
    }

    /// Asks the server to resend the whole screen.
    pub fn refresh(&self) {
        let _ = self.out.send(Outgoing::FullRefresh);
    }

    /// The UI finished drawing the last frame.
    pub fn frame_ack(&self) {
        self.ack.notify_one();
    }
}

fn pack_size(w: u16, h: u16) -> u32 {
    (w as u32) << 16 | h as u32
}

fn unpack_size(v: u32) -> (u16, u16) {
    ((v >> 16) as u16, v as u16)
}

async fn writer_loop<W>(wr: W, mut rx: mpsc::UnboundedReceiver<Outgoing>, size: Arc<AtomicU32>)
where
    W: AsyncWrite + Unpin,
{
    let mut wr = BufWriter::new(wr);
    let encode = |m| match m {
        Outgoing::Bytes(b) => b,
        Outgoing::FullRefresh => {
            let (w, h) = unpack_size(size.load(Ordering::Relaxed));
            proto::update_request(false, 0, 0, w, h)
        }
    };
    while let Some(first) = rx.recv().await {
        let mut msg = Some(first);
        // Batch whatever is already queued into a single flush.
        while let Some(m) = msg.take().or_else(|| rx.try_recv().ok()) {
            if wr.write_all(&encode(m)).await.is_err() {
                return;
            }
        }
        if wr.flush().await.is_err() {
            return;
        }
    }
}

async fn reader_loop<R, S>(
    mut rd: R,
    init: ServerInit,
    sink: S,
    out: mpsc::UnboundedSender<Outgoing>,
    ack: Arc<Notify>,
    size: Arc<AtomicU32>,
) where
    R: AsyncRead + Unpin,
    S: Sink,
{
    let name = init.name;
    let result: rfb::Result<()> = async {
        let mut fb = Framebuffer::new(init.width, init.height)?;
        let mut decoder = Decoder::new();
        sink.message(encode_event(&Event::Resize {
            width: fb.width(),
            height: fb.height(),
            name: name.clone(),
        }));
        loop {
            match decoder.read_message(&mut rd, &mut fb).await? {
                ServerMessage::FramebufferUpdate(update) => {
                    if update.resized {
                        size.store(pack_size(fb.width(), fb.height()), Ordering::Relaxed);
                        sink.message(encode_event(&Event::Resize {
                            width: fb.width(),
                            height: fb.height(),
                            name: name.clone(),
                        }));
                    }
                    if let Some(cursor) = &update.cursor {
                        sink.message(encode_cursor(cursor));
                    }
                    if !update.dirty.is_empty() {
                        sink.message(encode_rects(&fb, &update.dirty));
                        // Flow control: don't outrun the UI.
                        let _ = tokio::time::timeout(ACK_TIMEOUT, ack.notified()).await;
                    }
                    let next = if update.resized {
                        Outgoing::FullRefresh
                    } else {
                        Outgoing::Bytes(proto::update_request(true, 0, 0, fb.width(), fb.height()))
                    };
                    out.send(next).map_err(|_| rfb::Error::ConnectionLost)?;
                }
                ServerMessage::Bell => sink.message(encode_event(&Event::Bell)),
                ServerMessage::CutText(text) => sink.cut_text(text),
                ServerMessage::SetColourMap => {}
            }
        }
    }
    .await;
    let reason = result.err().map(|e| {
        log::warn!("session ended: {e}");
        e.info()
    });
    sink.message(encode_event(&Event::Disconnected { reason }));
}

pub fn encode_event(ev: &Event) -> Vec<u8> {
    let mut m = vec![MSG_EVENT];
    serde_json::to_writer(&mut m, ev).expect("event serializes");
    m
}

pub fn encode_rects(fb: &Framebuffer, dirty: &[Rect]) -> Vec<u8> {
    let bbox;
    let rects = if dirty.len() > MAX_RECTS_PER_FRAME {
        bbox = [dirty.iter().skip(1).fold(dirty[0], |acc, r| acc.union(r))];
        &bbox[..]
    } else {
        dirty
    };
    let size: usize = rects.iter().map(|r| 8 + r.area() * 4).sum();
    let mut m = Vec::with_capacity(5 + size);
    m.push(MSG_RECTS);
    m.extend_from_slice(&(rects.len() as u32).to_le_bytes());
    for r in rects {
        for v in [r.x, r.y, r.w, r.h] {
            m.extend_from_slice(&v.to_le_bytes());
        }
        fb.extract(*r, &mut m);
    }
    m
}

pub fn encode_cursor(c: &CursorShape) -> Vec<u8> {
    let mut m = Vec::with_capacity(9 + c.rgba.len());
    m.push(MSG_CURSOR);
    for v in [c.hot_x, c.hot_y, c.width, c.height] {
        m.extend_from_slice(&v.to_le_bytes());
    }
    m.extend_from_slice(&c.rgba);
    m
}

#[cfg(test)]
mod tests;
