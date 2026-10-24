//! End-to-end test against a scripted in-memory VNC server.

use flate2::{Compress, Compression, FlushCompress};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf};
use tokio::sync::mpsc;

use super::*;
use crate::rfb::auth;
use crate::rfb::proto::{PixelFormat, encoding};

const W: u16 = 64;
const H: u16 = 48;

enum Out {
    Msg(Vec<u8>),
    Cut(String),
}

struct TestSink(mpsc::UnboundedSender<Out>);

impl Sink for TestSink {
    fn message(&self, bytes: Vec<u8>) {
        let _ = self.0.send(Out::Msg(bytes));
    }
    fn cut_text(&self, text: String) {
        let _ = self.0.send(Out::Cut(text));
    }
}

/// Reference image the client should end up with.
struct Model {
    w: usize,
    px: Vec<[u8; 4]>,
}

impl Model {
    fn new(w: u16, h: u16) -> Self {
        Self {
            w: w as usize,
            px: vec![[0, 0, 0, 255]; w as usize * h as usize],
        }
    }
    fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        self.px[y * self.w + x] = [c[0], c[1], c[2], 255];
    }
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, c: [u8; 3]) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, c);
            }
        }
    }
    fn get(&self, x: usize, y: usize) -> [u8; 3] {
        let p = self.px[y * self.w + x];
        [p[0], p[1], p[2]]
    }
}

/// Parsed client-to-server message (type byte + payload).
#[derive(Debug, PartialEq)]
enum ClientMsg {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<i32>),
    UpdateRequest {
        incremental: bool,
        w: u16,
        h: u16,
    },
    Key {
        down: bool,
        keysym: u32,
    },
    Pointer {
        mask: u8,
        x: u16,
        y: u16,
    },
    CutText(Vec<u8>),
    /// Extended clipboard message: flags and the rest of the payload.
    ExtendedCut(u32, Vec<u8>),
    SetDesktopSize {
        w: u16,
        h: u16,
        screen_id: u32,
    },
}

async fn read_client_messages(
    mut rd: ReadHalf<DuplexStream>,
    tx: mpsc::UnboundedSender<ClientMsg>,
) {
    while let Ok(t) = rd.read_u8().await {
        let msg = match t {
            0 => {
                let mut b = [0u8; 19];
                rd.read_exact(&mut b).await.unwrap();
                ClientMsg::SetPixelFormat(PixelFormat::from_bytes(b[3..].try_into().unwrap()))
            }
            2 => {
                rd.read_u8().await.unwrap();
                let n = rd.read_u16().await.unwrap();
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(rd.read_i32().await.unwrap());
                }
                ClientMsg::SetEncodings(v)
            }
            3 => {
                let incremental = rd.read_u8().await.unwrap() != 0;
                let (_x, _y) = (rd.read_u16().await.unwrap(), rd.read_u16().await.unwrap());
                let (w, h) = (rd.read_u16().await.unwrap(), rd.read_u16().await.unwrap());
                ClientMsg::UpdateRequest { incremental, w, h }
            }
            4 => {
                let down = rd.read_u8().await.unwrap() != 0;
                rd.read_u16().await.unwrap();
                ClientMsg::Key {
                    down,
                    keysym: rd.read_u32().await.unwrap(),
                }
            }
            5 => {
                let mask = rd.read_u8().await.unwrap();
                ClientMsg::Pointer {
                    mask,
                    x: rd.read_u16().await.unwrap(),
                    y: rd.read_u16().await.unwrap(),
                }
            }
            6 => {
                let mut pad = [0u8; 3];
                rd.read_exact(&mut pad).await.unwrap();
                let len = rd.read_i32().await.unwrap();
                let mut v = vec![0u8; len.unsigned_abs() as usize];
                rd.read_exact(&mut v).await.unwrap();
                if len < 0 {
                    let flags = u32::from_be_bytes(v[..4].try_into().unwrap());
                    ClientMsg::ExtendedCut(flags, v[4..].to_vec())
                } else {
                    ClientMsg::CutText(v)
                }
            }
            251 => {
                rd.read_u8().await.unwrap();
                let (w, h) = (rd.read_u16().await.unwrap(), rd.read_u16().await.unwrap());
                assert_eq!(rd.read_u8().await.unwrap(), 1, "one screen");
                rd.read_u8().await.unwrap();
                let screen_id = rd.read_u32().await.unwrap();
                let mut rest = [0u8; 12];
                rd.read_exact(&mut rest).await.unwrap();
                ClientMsg::SetDesktopSize { w, h, screen_id }
            }
            t => panic!("unexpected client message {t}"),
        };
        if tx.send(msg).is_err() {
            return;
        }
    }
}

/// Builder for server messages (RGBX pixels on the wire).
#[derive(Default)]
struct Wire(Vec<u8>);

impl Wire {
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn px(&mut self, c: [u8; 3]) -> &mut Self {
        self.bytes(&[c[0], c[1], c[2], 0])
    }
    fn rect(&mut self, x: u16, y: u16, w: u16, h: u16, enc: i32) -> &mut Self {
        self.u16(x).u16(y).u16(w).u16(h).bytes(&enc.to_be_bytes())
    }
}

fn zlib_chunk(z: &mut Compress, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2 + 1024);
    let before = z.total_in();
    z.compress_vec(data, &mut out, FlushCompress::Sync).unwrap();
    assert_eq!((z.total_in() - before) as usize, data.len());
    out
}

fn run_bytes(mut len: usize) -> Vec<u8> {
    len -= 1;
    let mut v = vec![255; len / 255];
    v.push((len % 255) as u8);
    v
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];
const YELLOW: [u8; 3] = [255, 255, 0];
const CYAN: [u8; 3] = [0, 255, 255];
const GREY: [u8; 3] = [128, 128, 128];

fn raw_colour(x: usize, y: usize) -> [u8; 3] {
    [(x * 30) as u8, (y * 30) as u8, 100]
}

/// Builds a FramebufferUpdate exercising every encoding, and the expected image.
fn scripted_update() -> (Vec<u8>, Model) {
    let mut m = Model::new(W, H);
    let mut w = Wire::default();
    let mut z = Compress::new(Compression::default(), true);
    w.u8(0).u8(0).u16(10);

    // Raw (0,0,8,8)
    w.rect(0, 0, 8, 8, encoding::RAW);
    for y in 0..8 {
        for x in 0..8 {
            w.px(raw_colour(x, y));
            m.set(x, y, raw_colour(x, y));
        }
    }

    // RRE (8,0,16,16)
    w.rect(8, 0, 16, 16, encoding::RRE).u32(2).px(RED);
    w.px(BLUE).u16(2).u16(2).u16(4).u16(4);
    w.px(CYAN).u16(10).u16(10).u16(6).u16(6);
    m.fill(8, 0, 16, 16, RED);
    m.fill(10, 2, 4, 4, BLUE);
    m.fill(18, 10, 6, 6, CYAN);

    // Hextile (24,0,20,20): tiles 16x16, 4x16, 16x4, 4x4
    w.rect(24, 0, 20, 20, encoding::HEXTILE);
    w.u8(2 | 4 | 8).px(GREEN).px(WHITE).u8(1).u8(0x11).u8(0x21);
    m.fill(24, 0, 16, 16, GREEN);
    m.fill(25, 1, 3, 2, WHITE);
    w.u8(1);
    for y in 0..16 {
        for x in 0..4 {
            w.px(raw_colour(x, y));
            m.set(40 + x, y, raw_colour(x, y));
        }
    }
    w.u8(2 | 8 | 16).px(YELLOW).u8(2);
    w.px(RED).u8(0x00).u8(0x11);
    w.px(BLUE).u8(0x51).u8(0x00);
    m.fill(24, 16, 16, 4, YELLOW);
    m.fill(24, 16, 2, 2, RED);
    m.set(29, 17, BLUE);
    w.u8(0); // same background
    m.fill(40, 16, 4, 4, YELLOW);

    let mut zrle = |w: &mut Wire, x, y, rw, rh, tile: Vec<u8>| {
        let data = zlib_chunk(&mut z, &tile);
        w.rect(x, y, rw, rh, encoding::ZRLE)
            .u32(data.len() as u32)
            .bytes(&data);
    };

    // ZRLE solid (0,8,8,12)
    zrle(&mut w, 0, 8, 8, 12, vec![1, 10, 20, 30]);
    m.fill(0, 8, 8, 12, [10, 20, 30]);

    // ZRLE raw (8,16,16,4)
    let mut t = vec![0];
    for y in 0..4 {
        for x in 0..16 {
            t.extend_from_slice(&raw_colour(x, y));
            m.set(8 + x, 16 + y, raw_colour(x, y));
        }
    }
    zrle(&mut w, 8, 16, 16, 4, t);

    // ZRLE packed palette, 3 colours / 2 bits, width 30 to exercise row padding (0,20,30,28)
    let palette = [RED, GREEN, BLUE];
    let mut t = vec![3];
    palette.iter().for_each(|c| t.extend_from_slice(c));
    for y in 0..28 {
        let mut row = [0u8; 8];
        for x in 0..30 {
            let idx = (x + 2 * y) % 3;
            row[x / 4] |= (idx as u8) << (6 - 2 * (x % 4));
            m.set(x, 20 + y, palette[idx]);
        }
        t.extend_from_slice(&row);
    }
    zrle(&mut w, 0, 20, 30, 28, t);

    // ZRLE plain RLE (30,20,34,28): 300 white then grey
    let mut t = vec![128];
    t.extend_from_slice(&WHITE);
    t.extend(run_bytes(300));
    t.extend_from_slice(&GREY);
    t.extend(run_bytes(34 * 28 - 300));
    for i in 0..34 * 28 {
        m.set(30 + i % 34, 20 + i / 34, if i < 300 { WHITE } else { GREY });
    }
    zrle(&mut w, 30, 20, 34, 28, t);

    // ZRLE palette RLE (44,0,20,20): one cyan, 10 yellow, then red
    let mut t = vec![128 + 3];
    [CYAN, YELLOW, RED]
        .iter()
        .for_each(|c| t.extend_from_slice(c));
    t.push(0);
    t.push(0x81);
    t.extend(run_bytes(10));
    t.push(0x82);
    t.extend(run_bytes(400 - 11));
    for i in 0..400 {
        m.set(
            44 + i % 20,
            i / 20,
            match i {
                0 => CYAN,
                1..=10 => YELLOW,
                _ => RED,
            },
        );
    }
    zrle(&mut w, 44, 0, 20, 20, t);

    // CopyRect: raw block copied to the bottom-right corner
    w.rect(56, 40, 8, 8, encoding::COPY_RECT).u16(0).u16(0);
    for y in 0..8 {
        for x in 0..8 {
            m.set(56 + x, 40 + y, raw_colour(x, y));
        }
    }

    // Cursor 2x2, hotspot (1,1), diagonal mask
    w.rect(1, 1, 2, 2, encoding::CURSOR)
        .px(RED)
        .px(GREEN)
        .px(BLUE)
        .px(WHITE)
        .u8(0x80)
        .u8(0x40);

    (w.0, m)
}

/// Applies MSG_RECTS messages the way the UI does.
fn apply_rects(msg: &[u8], img: &mut [u8], img_w: usize) {
    assert_eq!(msg[0], MSG_RECTS);
    let count = u32::from_le_bytes(msg[1..5].try_into().unwrap());
    let mut p = 5;
    let rd = |p: usize| u16::from_le_bytes([msg[p], msg[p + 1]]) as usize;
    for _ in 0..count {
        let (x, y, w, h) = (rd(p), rd(p + 2), rd(p + 4), rd(p + 6));
        p += 8;
        for row in 0..h {
            let dst = ((y + row) * img_w + x) * 4;
            img[dst..dst + w * 4].copy_from_slice(&msg[p..p + w * 4]);
            p += w * 4;
        }
    }
    assert_eq!(p, msg.len());
}

fn event(msg: &[u8]) -> serde_json::Value {
    assert_eq!(msg[0], MSG_EVENT, "expected an event");
    serde_json::from_slice(&msg[1..]).unwrap()
}

async fn next_msg(rx: &mut mpsc::UnboundedReceiver<Out>) -> Vec<u8> {
    match rx.recv().await.expect("sink closed") {
        Out::Msg(m) => m,
        Out::Cut(t) => panic!("unexpected cut text {t:?}"),
    }
}

#[tokio::test]
async fn full_session_against_scripted_server() {
    let (client, server) = tokio::io::duplex(1 << 20);
    let (srv_rd, mut srv_wr) = tokio::io::split(server);

    // Server side of the handshake runs concurrently with Session::start.
    let handshake = tokio::spawn(async move {
        let mut srv_rd = srv_rd;
        srv_wr.write_all(b"RFB 003.008\n").await.unwrap();
        let mut v = [0u8; 12];
        srv_rd.read_exact(&mut v).await.unwrap();
        assert_eq!(&v, b"RFB 003.008\n");
        srv_wr.write_all(&[2, 1, 2]).await.unwrap();
        assert_eq!(
            srv_rd.read_u8().await.unwrap(),
            2,
            "VNC auth chosen when a password is given"
        );
        let challenge = [0x5Au8; 16];
        srv_wr.write_all(&challenge).await.unwrap();
        let mut resp = [0u8; 16];
        srv_rd.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, auth::challenge_response("pass", &challenge));
        srv_wr.write_u32(0).await.unwrap();
        assert_eq!(srv_rd.read_u8().await.unwrap(), 1, "shared flag");
        let mut pf = PixelFormat::RGBX;
        pf.bits_per_pixel = 16; // the client must override it
        srv_wr.write_all(&[0, W as u8, 0, H as u8]).await.unwrap();
        srv_wr.write_all(&pf.to_bytes()).await.unwrap();
        srv_wr.write_u32(4).await.unwrap();
        srv_wr.write_all(b"mock").await.unwrap();
        (srv_rd, srv_wr)
    });

    let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();
    let opts = SecurityOptions {
        password: Some("pass".into()),
        allow_vencrypt: true,
        ..Default::default()
    };
    let session = Session::start(Box::new(client), &opts, true, TestSink(sink_tx))
        .await
        .unwrap();
    let (srv_rd, mut srv_wr) = handshake.await.unwrap();
    let (cmsg_tx, mut cmsgs) = mpsc::unbounded_channel();
    tokio::spawn(read_client_messages(srv_rd, cmsg_tx));

    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::SetPixelFormat(PixelFormat::RGBX)
    );
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::SetEncodings(proto::encodings(Quality::Lossless))
    );
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::UpdateRequest {
            incremental: false,
            w: W,
            h: H
        }
    );

    assert_eq!(
        event(&next_msg(&mut sink_rx).await),
        serde_json::json!({"type": "connected", "security": "vnc", "encrypted": false, "tunnel": false})
    );
    let ev = event(&next_msg(&mut sink_rx).await);
    assert_eq!(
        ev,
        serde_json::json!({"type": "resize", "width": W, "height": H, "name": "mock"})
    );

    // One update with every encoding.
    let (update, model) = scripted_update();
    srv_wr.write_all(&update).await.unwrap();

    let cursor = next_msg(&mut sink_rx).await;
    assert_eq!(cursor[0], MSG_CURSOR);
    assert_eq!(&cursor[1..9], &[1, 0, 1, 0, 2, 0, 2, 0]);
    assert_eq!(
        &cursor[9..],
        &[
            255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 0, 255, 255, 255, 255
        ]
    );

    let rects = next_msg(&mut sink_rx).await;
    let mut img = vec![0u8; W as usize * H as usize * 4];
    apply_rects(&rects, &mut img, W as usize);
    session.frame_ack();
    for y in 0..H as usize {
        for x in 0..W as usize {
            let got = &img[(y * W as usize + x) * 4..][..4];
            let want = model.get(x, y);
            assert_eq!(got, &[want[0], want[1], want[2], 255], "pixel ({x},{y})");
        }
    }
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::UpdateRequest {
            incremental: true,
            w: W,
            h: H
        }
    );

    // Input goes out in order.
    session.input(&[
        InputEvent::Pointer {
            x: 3,
            y: 4,
            mask: 1,
        },
        InputEvent::Key {
            keysym: 0x61,
            down: true,
        },
    ]);
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::Pointer {
            mask: 1,
            x: 3,
            y: 4
        }
    );
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::Key {
            down: true,
            keysym: 0x61
        }
    );
    session.send_cut_text("€uro é");
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::CutText(b"?uro \xE9".to_vec())
    );

    // Bell and server clipboard.
    let mut w = Wire::default();
    w.u8(2).u8(3).bytes(&[0, 0, 0]).u32(5).bytes(b"h\xE9llo");
    srv_wr.write_all(&w.0).await.unwrap();
    assert_eq!(
        event(&next_msg(&mut sink_rx).await),
        serde_json::json!({"type": "bell"})
    );
    match sink_rx.recv().await.unwrap() {
        Out::Cut(t) => assert_eq!(t, "héllo"),
        Out::Msg(_) => panic!("expected cut text"),
    }

    // Desktop resize, followed by a rectangle in the new geometry.
    let mut w = Wire::default();
    w.u8(0)
        .u8(0)
        .u16(2)
        .rect(0, 0, 32, 16, encoding::DESKTOP_SIZE);
    w.rect(0, 0, 32, 16, encoding::HEXTILE);
    w.u8(2).px(BLUE).u8(2).px(RED);
    srv_wr.write_all(&w.0).await.unwrap();
    assert_eq!(
        event(&next_msg(&mut sink_rx).await),
        serde_json::json!({"type": "resize", "width": 32, "height": 16, "name": "mock"})
    );
    let rects = next_msg(&mut sink_rx).await;
    let mut img = vec![0u8; 32 * 16 * 4];
    apply_rects(&rects, &mut img, 32);
    session.frame_ack();
    assert_eq!(&img[..4], &[0, 0, 255, 255]);
    assert_eq!(&img[(15 * 32 + 31) * 4..][..4], &[255, 0, 0, 255]);
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::UpdateRequest {
            incremental: false,
            w: 32,
            h: 16
        }
    );

    // A garbage message ends the session with a reason.
    srv_wr.write_all(&[42]).await.unwrap();
    let ev = event(&next_msg(&mut sink_rx).await);
    assert_eq!(ev["type"], "disconnected");
    assert_eq!(
        ev["reason"],
        serde_json::json!({"code": "unknownMessage", "detail": "42"})
    );
}

#[test]
fn many_rects_are_merged_into_a_bounding_box() {
    let fb = Framebuffer::new(100, 100).unwrap();
    let dirty: Vec<Rect> = (0..100).map(|i| Rect::new(i, i, 1, 1)).collect();
    let msg = encode_rects(&fb, &dirty);
    assert_eq!(u32::from_le_bytes(msg[1..5].try_into().unwrap()), 1);
    assert_eq!(&msg[5..13], &[0, 0, 0, 0, 100, 0, 100, 0]);
    assert_eq!(msg.len(), 13 + 100 * 100 * 4);
}

/// Server side of an RFB 3.8 handshake without authentication.
async fn serve_handshake(
    mut srv_rd: ReadHalf<DuplexStream>,
    mut srv_wr: tokio::io::WriteHalf<DuplexStream>,
) -> (ReadHalf<DuplexStream>, tokio::io::WriteHalf<DuplexStream>) {
    srv_wr.write_all(b"RFB 003.008\n").await.unwrap();
    let mut v = [0u8; 12];
    srv_rd.read_exact(&mut v).await.unwrap();
    srv_wr.write_all(&[1, 1]).await.unwrap();
    assert_eq!(srv_rd.read_u8().await.unwrap(), 1);
    srv_wr.write_u32(0).await.unwrap();
    srv_rd.read_u8().await.unwrap();
    srv_wr.write_all(&[0, W as u8, 0, H as u8]).await.unwrap();
    srv_wr
        .write_all(&PixelFormat::RGBX.to_bytes())
        .await
        .unwrap();
    srv_wr.write_u32(1).await.unwrap();
    srv_wr.write_all(b"x").await.unwrap();
    (srv_rd, srv_wr)
}

/// Payload of a server extended clipboard message.
fn server_extended_cut(flags: u32, data: &[u8]) -> Vec<u8> {
    let mut w = Wire::default();
    w.u8(3)
        .bytes(&[0, 0, 0])
        .bytes(&(-((4 + data.len()) as i32)).to_be_bytes())
        .u32(flags)
        .bytes(data);
    w.0
}

#[tokio::test]
async fn extended_clipboard_and_remote_resize() {
    use crate::rfb::clipboard as xc;

    let (client, server) = tokio::io::duplex(1 << 20);
    let (srv_rd, srv_wr) = tokio::io::split(server);
    let handshake = tokio::spawn(serve_handshake(srv_rd, srv_wr));
    let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();
    let session = Session::start(
        Box::new(client),
        &SecurityOptions::default(),
        true,
        TestSink(sink_tx),
    )
    .await
    .unwrap();
    let (srv_rd, mut srv_wr) = handshake.await.unwrap();
    let (cmsg_tx, mut cmsgs) = mpsc::unbounded_channel();
    tokio::spawn(read_client_messages(srv_rd, cmsg_tx));
    for _ in 0..3 {
        cmsgs.recv().await.unwrap(); // pixel format, encodings, full refresh
    }
    assert_eq!(event(&next_msg(&mut sink_rx).await)["type"], "connected");
    assert_eq!(event(&next_msg(&mut sink_rx).await)["type"], "resize");

    // Without the extension, the legacy (Latin-1) message is used.
    session.send_cut_text("a");
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::CutText(b"a".to_vec())
    );

    // The server announces the extension; the client answers with its caps.
    srv_wr
        .write_all(&server_extended_cut(
            xc::CAPS | xc::TEXT,
            &xc::MAX_TEXT.to_be_bytes(),
        ))
        .await
        .unwrap();
    match cmsgs.recv().await.unwrap() {
        ClientMsg::ExtendedCut(flags, _) => assert_ne!(flags & xc::CAPS, 0),
        m => panic!("expected caps, got {m:?}"),
    }

    // Local text: notify, then provide on request.
    session.send_cut_text("€ 🎉");
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::ExtendedCut(xc::NOTIFY | xc::TEXT, vec![])
    );
    srv_wr
        .write_all(&server_extended_cut(xc::REQUEST | xc::TEXT, &[]))
        .await
        .unwrap();
    let ClientMsg::ExtendedCut(flags, data) = cmsgs.recv().await.unwrap() else {
        panic!("expected provide");
    };
    let mut all = flags.to_be_bytes().to_vec();
    all.extend(data);
    assert_eq!(
        xc::parse(&all).unwrap(),
        xc::Message::Provide(Some("€ 🎉".into()))
    );

    // Remote text: notify -> the client requests -> provide reaches the sink.
    srv_wr
        .write_all(&server_extended_cut(xc::NOTIFY | xc::TEXT, &[]))
        .await
        .unwrap();
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::ExtendedCut(xc::REQUEST | xc::TEXT, vec![])
    );
    let provide = xc::provide_text("ok\nUTF-8 ✓");
    srv_wr
        .write_all(&server_extended_cut(
            u32::from_be_bytes(provide[8..12].try_into().unwrap()),
            &provide[12..],
        ))
        .await
        .unwrap();
    match sink_rx.recv().await.unwrap() {
        Out::Cut(t) => assert_eq!(t, "ok\nUTF-8 ✓"),
        Out::Msg(m) => panic!("expected cut text, got {m:?}"),
    }
    // Text the server already has is not sent back.
    assert!(!session.send_cut_text("ok\nUTF-8 ✓"));

    // No ExtendedDesktopSize yet: resize requests are dropped.
    session.set_desktop_size(10, 10);
    // The server reports its layout (same size, screen id 7)...
    let mut w = Wire::default();
    w.u8(0).u8(0).u16(1);
    w.rect(0, 0, W, H, encoding::EXTENDED_DESKTOP_SIZE)
        .u8(1)
        .bytes(&[0, 0, 0])
        .u32(7)
        .u16(0)
        .u16(0)
        .u16(W)
        .u16(H)
        .u32(0);
    srv_wr.write_all(&w.0).await.unwrap();
    assert_eq!(
        event(&next_msg(&mut sink_rx).await),
        serde_json::json!({"type": "remoteResize"})
    );
    assert!(matches!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::UpdateRequest {
            incremental: true,
            ..
        }
    ));
    // ...then resize requests go out with its screen id.
    session.set_desktop_size(100, 50);
    assert_eq!(
        cmsgs.recv().await.unwrap(),
        ClientMsg::SetDesktopSize {
            w: 100,
            h: 50,
            screen_id: 7
        }
    );
    // The server applies it: the framebuffer follows.
    let mut w = Wire::default();
    w.u8(0).u8(0).u16(1);
    w.rect(1, 0, 100, 50, encoding::EXTENDED_DESKTOP_SIZE)
        .u8(1)
        .bytes(&[0, 0, 0])
        .u32(7)
        .u16(0)
        .u16(0)
        .u16(100)
        .u16(50)
        .u32(0);
    srv_wr.write_all(&w.0).await.unwrap();
    assert_eq!(
        event(&next_msg(&mut sink_rx).await),
        serde_json::json!({"type": "resize", "width": 100, "height": 50, "name": "x"})
    );
}

#[tokio::test]
async fn vencrypt_x509_with_certificate_pinning() {
    use std::sync::Arc;

    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

    use crate::rfb::security::{VENCRYPT, vencrypt};
    use crate::rfb::{Error, tls};

    let cert = rcgen::generate_simple_self_signed(vec!["vnc.test".into()]).unwrap();
    let der = cert.cert.der().clone();
    let fp = tls::fingerprint(&der);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![der], key)
    .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    // Server: VeNCrypt 0.2 offering X509Vnc, then VNC auth inside TLS.
    let serve = |acceptor: TlsAcceptor, server: DuplexStream| async move {
        let mut s = server;
        s.write_all(b"RFB 003.008\n").await.unwrap();
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.unwrap();
        s.write_all(&[1, VENCRYPT as u8]).await.unwrap();
        assert_eq!(s.read_u8().await.unwrap(), VENCRYPT as u8);
        s.write_all(&[0, 2]).await.unwrap();
        let mut ver = [0u8; 2];
        s.read_exact(&mut ver).await.unwrap();
        assert_eq!(ver, [0, 2]);
        s.write_all(&[0, 2]).await.unwrap(); // OK, 2 subtypes
        s.write_u32(vencrypt::TLS_VNC).await.unwrap();
        s.write_u32(vencrypt::X509_VNC).await.unwrap();
        assert_eq!(s.read_u32().await.unwrap(), vencrypt::X509_VNC);
        s.write_u8(1).await.unwrap();
        let Ok(mut tls) = acceptor.accept(s).await else {
            return None; // the client refused the certificate
        };
        let challenge = [3u8; 16];
        tls.write_all(&challenge).await.unwrap();
        let mut resp = [0u8; 16];
        tls.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, auth::challenge_response("pw", &challenge));
        tls.write_u32(0).await.unwrap();
        assert_eq!(tls.read_u8().await.unwrap(), 0, "exclusive");
        tls.write_all(&[0, 4, 0, 4]).await.unwrap();
        tls.write_all(&PixelFormat::RGBX.to_bytes()).await.unwrap();
        tls.write_u32(3).await.unwrap();
        tls.write_all(b"tls").await.unwrap();
        tls.flush().await.unwrap();
        Some(tls)
    };
    let mut opts = SecurityOptions {
        password: Some("pw".into()),
        host: "vnc.test".into(),
        allow_vencrypt: true,
        ..Default::default()
    };

    // Self-signed and not trusted yet: refused, with its fingerprint.
    let (client, server) = tokio::io::duplex(1 << 16);
    let srv = tokio::spawn(serve(acceptor.clone(), server));
    let err = rfb::handshake(Box::new(client), &opts, false)
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, Error::UntrustedCertificate(f) if *f == fp),
        "{err:?}"
    );
    srv.abort();

    // Once trusted, the session is encrypted.
    opts.trusted_cert = Some(fp);
    let (client, server) = tokio::io::duplex(1 << 16);
    let srv = tokio::spawn(serve(acceptor, server));
    let (_stream, init, security) = rfb::handshake(Box::new(client), &opts, false)
        .await
        .unwrap();
    assert_eq!(security, Security::X509Vnc);
    assert!(security.encrypted());
    assert_eq!((init.width, init.name.as_str()), (4, "tls"));
    drop(srv);
}

/// Against a real server, e.g.
/// `VNC_LIVE='{"host":"localhost","port":5901,"password":"secret","shared":true}'
/// cargo test -p vncviewer live -- --ignored --nocapture`.
/// An untrusted certificate or SSH host key is accepted (and reported).
#[tokio::test]
#[ignore]
async fn live_server() {
    let Ok(json) = std::env::var("VNC_LIVE") else {
        panic!("set VNC_LIVE to the connection parameters (JSON)");
    };
    let mut params: ConnectParams = serde_json::from_str(&json).unwrap();
    let (sink_tx, mut sink_rx) = mpsc::unbounded_channel();
    let session = loop {
        match Session::connect(&params, TestSink(sink_tx.clone())).await {
            Ok(s) => break s,
            Err(rfb::Error::UntrustedCertificate(fp)) => {
                eprintln!("trusting certificate {fp}");
                params.trusted_cert = Some(fp);
            }
            Err(rfb::Error::SshHostKeyUnknown(fp)) => {
                eprintln!("trusting SSH host key {fp}");
                params.ssh.as_mut().unwrap().trusted_key = Some(fp);
            }
            Err(e) => panic!("connection failed: {e}"),
        }
    };
    let connected = event(&next_msg(&mut sink_rx).await);
    eprintln!("{connected}");
    let resize = event(&next_msg(&mut sink_rx).await);
    eprintln!("{resize}");
    assert_eq!(resize["type"], "resize");
    let (w, h) = (
        resize["width"].as_u64().unwrap() as u16,
        resize["height"].as_u64().unwrap() as u16,
    );
    let mut remote_resize = false;
    let mut resized = false;
    let mut pixels = false;
    let mut img = vec![0u8; w as usize * h as usize * 4];
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while let Ok(Some(out)) = tokio::time::timeout_at(deadline, sink_rx.recv()).await {
        let Out::Msg(m) = out else { continue };
        eprintln!("message kind {} ({} bytes)", m[0], m.len());
        match m[0] {
            MSG_RECTS => {
                if !resized {
                    apply_rects(&m, &mut img, w as usize);
                    let lit = img
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .filter(|p| p[..3] != [0, 0, 0])
                        .count();
                    eprintln!("frame: {lit} non-black pixels");
                    pixels |= lit > 0;
                    if pixels && remote_resize {
                        session.set_desktop_size(w - 100, h - 100);
                    }
                }
                session.frame_ack();
                if resized {
                    break;
                }
            }
            MSG_EVENT => {
                let ev = event(&m);
                eprintln!("{ev}");
                assert_ne!(ev["type"], "disconnected");
                if ev["type"] == "remoteResize" {
                    remote_resize = true;
                }
                if ev["type"] == "resize" {
                    assert_eq!(ev["width"], w - 100);
                    resized = true;
                }
            }
            _ => {}
        }
    }
    assert!(pixels, "no visible pixels received");
    eprintln!("remote resize supported: {remote_resize}");
}
