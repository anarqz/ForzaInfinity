//! Minimal WebSocket server (RFC 6455) for the web UI: the handshake, unmasked text frames
//! from the server, and reading the client's frames only to answer ping and close.
//! Every client gets every broadcast; the UI filters by xuid.

use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::Value;

/// RFC 6455 section 1.3. Checked against the RFC's example (see the test below).
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Default)]
pub struct Hub {
    clients: Mutex<Vec<(u64, TcpStream)>>,
    next: AtomicU64,
}

impl Hub {
    pub fn broadcast(&self, message: &Value) {
        let frame = text_frame(&message.to_string());
        let mut clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());
        clients.retain_mut(|(_, stream)| stream.write_all(&frame).and_then(|_| stream.flush()).is_ok());
    }

    pub fn count(&self) -> usize {
        self.clients.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Takes over a connection whose request asked for an upgrade. Blocks until it closes.
    pub fn serve(&self, mut stream: TcpStream, key: &str, hello: &Value) -> std::io::Result<()> {
        let accept = accept_key(key);
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        )?;
        stream.write_all(&text_frame(&hello.to_string()))?;
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;

        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).push((id, stream.try_clone()?));
        let result = read_until_close(&mut stream);
        self.clients.lock().unwrap_or_else(|p| p.into_inner()).retain(|(i, _)| *i != id);
        result
    }
}

fn text_frame(text: &str) -> Vec<u8> {
    let payload = text.as_bytes();
    let mut frame = vec![0x81]; // FIN + text
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n <= 0xffff => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

fn read_until_close(stream: &mut TcpStream) -> std::io::Result<()> {
    loop {
        let mut head = [0u8; 2];
        if stream.read_exact(&mut head).is_err() {
            return Ok(());
        }
        let opcode = head[0] & 0x0f;
        let masked = head[1] & 0x80 != 0;
        let mut len = (head[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        if len > 1 << 20 {
            return Ok(()); // the UI never sends anything that big
        }
        let mut mask = [0u8; 4];
        if masked {
            stream.read_exact(&mut mask)?;
        }
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload)?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => {
                let _ = stream.write_all(&[0x88, 0]);
                return Ok(());
            }
            0x9 => {
                // ping -> pong with the same payload (control frames are < 126 bytes)
                let mut pong = vec![0x8a, payload.len() as u8];
                pong.extend_from_slice(&payload);
                stream.write_all(&pong)?;
            }
            _ => {}
        }
    }
}

pub fn accept_key(key: &str) -> String {
    let mut sha = sha1_smol::Sha1::new();
    sha.update(key.trim().as_bytes());
    sha.update(GUID.as_bytes());
    B64.encode(sha.digest().bytes())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc6455_example() {
        // RFC 6455 section 1.3: this key must give this accept value.
        assert_eq!(super::accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }
}