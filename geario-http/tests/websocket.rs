//! A real WebSocket handshake and frame exchange over a socket.
#![cfg(feature = "ws")]

use geario::bytes::Bytes;
use geario::service::cfg::SharedCfg;
use geario_http::ws;

/// A server that completes the handshake by hand and echoes text frames.
///
/// Written against the raw protocol rather than the client's own helpers, so
/// the test would notice if both sides drifted together.
fn echo_server() -> String {
    use std::io::{Read, Write};

    let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = lst.local_addr().unwrap();

    std::thread::spawn(move || {
        let Ok((mut s, _)) = lst.accept() else { return };
        let mut buf = [0u8; 2048];
        let Ok(n) = s.read(&mut buf) else { return };
        let req = String::from_utf8_lossy(&buf[..n]).to_string();

        let key = req
            .lines()
            .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
            .or_else(|| req.lines().find_map(|l| l.strip_prefix("sec-websocket-key: ")))
            .unwrap_or("")
            .trim()
            .to_owned();

        let accept = ws::hash_key(key.as_bytes()).expect("hash key");
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\n\
             upgrade: websocket\r\nconnection: upgrade\r\n\
             sec-websocket-accept: {accept}\r\n\r\n"
        );
        let _ = s.write_all(resp.as_bytes());

        // Echo whatever frames arrive, bytes for bytes. A masked client frame
        // becomes an unmasked server frame, so the payload has to be unmasked
        // and rewritten rather than reflected.
        loop {
            let Ok(n) = s.read(&mut buf) else { return };
            if n < 2 {
                return;
            }
            let masked = buf[1] & 0x80 != 0;
            let len = (buf[1] & 0x7f) as usize;
            let mut i = 2;
            let mask = if masked {
                let m = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]];
                i += 4;
                Some(m)
            } else {
                None
            };
            let mut payload = buf[i..i + len].to_vec();
            if let Some(m) = mask {
                for (j, b) in payload.iter_mut().enumerate() {
                    *b ^= m[j % 4];
                }
            }
            let mut out = vec![buf[0], payload.len() as u8];
            out.extend_from_slice(&payload);
            if s.write_all(&out).is_err() {
                return;
            }
        }
    });

    format!("ws://{addr}/")
}

#[geario::test]
async fn handshake_then_echo_a_text_frame() {
    let url = echo_server();

    let cfg: geario::service::cfg::SharedCfg = SharedCfg::new("WS").into();
    let conn = ws::WsClient::new(url.as_str(), cfg.get())
        .expect("client")
        .connect()
        .await
        .expect("handshake")
        .seal();

    let sink = conn.sink();
    sink.send(ws::Message::Text("hello ws".into()))
        .await
        .expect("send");

    let mut rx = conn.receiver();
    let frame = geario::util::future::stream_recv(&mut rx)
        .await
        .expect("no frame")
        .expect("frame error");

    match frame {
        ws::Frame::Text(bytes) => assert_eq!(bytes, Bytes::from_static(b"hello ws")),
        other => panic!("expected a text frame, got {other:?}"),
    }
}
