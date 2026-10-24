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

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::rfb::clipboard::{self as xclip, Message as ClipMessage};
use crate::rfb::proto::Quality;
use crate::rfb::{
    self, BoxStream, CursorShape, Decoder, Framebuffer, Rect, Security, SecurityOptions,
    ServerInit, ServerMessage, proto,
};
use crate::ssh::{self, SshParams};

pub const MSG_EVENT: u8 = 0;
pub const MSG_RECTS: u8 = 1;
pub const MSG_CURSOR: u8 = 2;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for the UI to draw a frame before asking for the next one.
const ACK_TIMEOUT: Duration = Duration::from_millis(500);
/// Above this many rectangles, a single bounding box is sent instead.
const MAX_RECTS_PER_FRAME: usize = 64;

/// Receives everything the session produces for the UI.
pub trait Sink: Send + Sync + 'static {
    fn message(&self, bytes: Vec<u8>);
    fn cut_text(&self, text: String);
}

/// Everything needed to open a session, as sent by the UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    pub shared: bool,
    /// SHA-256 fingerprint of a TLS certificate the user chose to trust.
    #[serde(default)]
    pub trusted_cert: Option<String>,
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub ssh: Option<SshParams>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// Sent once, before the first `resize`.
    Connected {
        security: Security,
        /// The whole session is encrypted (TLS or SSH tunnel).
        encrypted: bool,
        tunnel: bool,
    },
    /// The server can change its desktop size on request.
    RemoteResize,
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

/// State shared by the session handle and its reader task.
#[derive(Default)]
struct Shared {
    /// The server supports the extended clipboard for text.
    extended_clipboard: bool,
    /// Local clipboard text announced to the server, provided on request.
    local_text: Option<String>,
    /// Screen id from ExtendedDesktopSize, once the server announced it.
    screen_id: Option<u32>,
}

pub struct Session {
    out: mpsc::UnboundedSender<Outgoing>,
    ack: Arc<Notify>,
    shared: Arc<Mutex<Shared>>,
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
    /// Opens the connection (directly or through SSH), performs the handshake
    /// and starts the session.
    pub async fn connect<S: Sink>(p: &ConnectParams, sink: S) -> rfb::Result<Session> {
        let mut opts = SecurityOptions {
            username: p.username.clone().filter(|u| !u.is_empty()),
            password: p.password.clone().filter(|p| !p.is_empty()),
            host: p.host.clone(),
            trusted_cert: p.trusted_cert.clone(),
            allow_vencrypt: true,
        };
        let attempt = async {
            loop {
                let stream = open(p).await?;
                match rfb::handshake(stream, &opts, p.shared).await {
                    // Only anonymous TLS offered: retry with the other types.
                    Err(rfb::Error::VencryptUnusable(subtypes)) if opts.allow_vencrypt => {
                        log::info!("VeNCrypt unusable ({subtypes:?}), retrying without it");
                        opts.allow_vencrypt = false;
                    }
                    r => return r,
                }
            }
        };
        let (stream, init, security) = tokio::time::timeout(CONNECT_TIMEOUT, attempt)
            .await
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))??;
        Ok(Self::run(
            stream,
            init,
            security,
            p.ssh.is_some(),
            p.quality,
            sink,
        ))
    }

    /// Performs the handshake on an established stream and starts the session.
    #[cfg(test)]
    pub async fn start<S: Sink>(
        stream: BoxStream,
        opts: &SecurityOptions,
        shared: bool,
        sink: S,
    ) -> rfb::Result<Session> {
        let (stream, init, security) = rfb::handshake(stream, opts, shared).await?;
        Ok(Self::run(
            stream,
            init,
            security,
            false,
            Quality::Lossless,
            sink,
        ))
    }

    fn run<S: Sink>(
        stream: BoxStream,
        init: ServerInit,
        security: Security,
        tunnel: bool,
        quality: Quality,
        sink: S,
    ) -> Session {
        log::info!(
            "connected to {:?} ({}×{}), security {security:?}, server pixel format {:?}",
            init.name,
            init.width,
            init.height,
            init.pixel_format
        );
        sink.message(encode_event(&Event::Connected {
            security,
            encrypted: tunnel || security.encrypted(),
            tunnel,
        }));

        let (rd, wr) = tokio::io::split(stream);
        let (out, rx) = mpsc::unbounded_channel();
        let ack = Arc::new(Notify::new());
        let shared = Arc::new(Mutex::new(Shared::default()));
        let size = Arc::new(AtomicU32::new(pack_size(init.width, init.height)));

        let _ = out.send(Outgoing::Bytes(proto::set_pixel_format(
            &proto::PixelFormat::RGBX,
        )));
        let _ = out.send(Outgoing::Bytes(proto::set_encodings(&proto::encodings(
            quality,
        ))));
        let _ = out.send(Outgoing::FullRefresh);

        let writer = tokio::spawn(writer_loop(wr, rx, size.clone()));
        let reader = tokio::spawn(reader_loop(
            BufReader::with_capacity(1 << 18, rd),
            init,
            sink,
            Channels {
                out: out.clone(),
                ack: ack.clone(),
                shared: shared.clone(),
                size,
            },
        ));
        Session {
            out,
            ack,
            shared,
            tasks: [reader, writer],
        }
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
        let mut shared = self.shared.lock().unwrap();
        let msg = if shared.extended_clipboard {
            // Announce it; the server asks for the content when it wants it.
            shared.local_text = Some(text.to_owned());
            xclip::notify(true)
        } else {
            proto::client_cut_text(text)
        };
        let _ = self.out.send(Outgoing::Bytes(msg));
    }

    /// Asks the server to resize its desktop, if it supports it.
    pub fn set_desktop_size(&self, width: u16, height: u16) {
        if let Some(id) = self.shared.lock().unwrap().screen_id {
            let _ = self
                .out
                .send(Outgoing::Bytes(proto::set_desktop_size(width, height, id)));
        }
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

/// What the reader task shares with the rest of the session.
struct Channels {
    out: mpsc::UnboundedSender<Outgoing>,
    ack: Arc<Notify>,
    shared: Arc<Mutex<Shared>>,
    size: Arc<AtomicU32>,
}

impl Channels {
    fn send(&self, bytes: Vec<u8>) -> rfb::Result<()> {
        self.out
            .send(Outgoing::Bytes(bytes))
            .map_err(|_| rfb::Error::ConnectionLost)
    }
}

async fn reader_loop<R, S>(mut rd: R, init: ServerInit, sink: S, ch: Channels)
where
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
                    if let Some(layout) = update.desktop {
                        let first = ch
                            .shared
                            .lock()
                            .unwrap()
                            .screen_id
                            .replace(layout.screen_id)
                            .is_none();
                        if first {
                            sink.message(encode_event(&Event::RemoteResize));
                        }
                        if layout.reason == 1 && layout.status != 0 {
                            log::info!("desktop resize refused (status {})", layout.status);
                        }
                    }
                    if update.resized {
                        ch.size
                            .store(pack_size(fb.width(), fb.height()), Ordering::Relaxed);
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
                        let _ = tokio::time::timeout(ACK_TIMEOUT, ch.ack.notified()).await;
                    }
                    let next = if update.resized {
                        Outgoing::FullRefresh
                    } else {
                        Outgoing::Bytes(proto::update_request(true, 0, 0, fb.width(), fb.height()))
                    };
                    ch.out.send(next).map_err(|_| rfb::Error::ConnectionLost)?;
                }
                ServerMessage::Bell => sink.message(encode_event(&Event::Bell)),
                ServerMessage::CutText(text) => sink.cut_text(text),
                ServerMessage::Clipboard(msg) => handle_clipboard(msg, &sink, &ch)?,
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

fn handle_clipboard<S: Sink>(msg: ClipMessage, sink: &S, ch: &Channels) -> rfb::Result<()> {
    match msg {
        ClipMessage::Caps { text } => {
            ch.shared.lock().unwrap().extended_clipboard = text;
            if text {
                ch.send(xclip::caps())?;
            }
        }
        ClipMessage::Notify { text: true } => ch.send(xclip::request_text())?,
        ClipMessage::Notify { text: false } | ClipMessage::Request { text: false } => {}
        ClipMessage::Request { text: true } => {
            let text = ch.shared.lock().unwrap().local_text.clone();
            if let Some(text) = text {
                ch.send(xclip::provide_text(&text))?;
            }
        }
        ClipMessage::Peek => {
            let has_text = ch.shared.lock().unwrap().local_text.is_some();
            ch.send(xclip::notify(has_text))?;
        }
        ClipMessage::Provide(Some(text)) => sink.cut_text(text),
        ClipMessage::Provide(None) => {}
    }
    Ok(())
}

/// Opens the byte stream to the VNC server, through SSH if configured.
async fn open(p: &ConnectParams) -> rfb::Result<BoxStream> {
    if let Some(tunnel) = &p.ssh {
        return ssh::tunnel(tunnel, p.host.trim(), p.port).await;
    }
    let stream = TcpStream::connect((p.host.trim(), p.port)).await?;
    stream.set_nodelay(true)?;
    Ok(Box::new(stream))
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
