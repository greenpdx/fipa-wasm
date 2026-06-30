//! Authenticated, encrypted node transport (R2). A Noise XX handshake (mutual auth
//! + forward secrecy) runs before any data; the signed envelope (R1) rides inside.
//! The peer's static key is captured at handshake so a node can pin it (C2a).

use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::Path;

use snow::params::NoiseParams;
use snow::{Builder, TransportState};

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// One Noise message is at most 65535 bytes, so this caps a pre-handshake
/// allocation tightly (important on a 400 KB-RAM device) without rejecting any
/// legitimate frame (`send` caps the plaintext below this).
const MAX_BLOB: usize = 1 << 16; // 64 KiB
const MAX_PLAIN: usize = 65535 - 16; // one Noise message minus the AEAD tag

fn params() -> NoiseParams {
    PATTERN.parse().expect("valid noise params")
}
fn noise_err(e: snow::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("noise: {e}"))
}

fn write_blob(s: &mut impl Write, b: &[u8]) -> io::Result<()> {
    s.write_all(&(b.len() as u32).to_be_bytes())?;
    s.write_all(b)?;
    s.flush()
}
fn read_blob(s: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut l = [0u8; 4];
    s.read_exact(&mut l)?;
    let n = u32::from_be_bytes(l) as usize;
    if n > MAX_BLOB {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "blob too large"));
    }
    let mut b = vec![0u8; n];
    s.read_exact(&mut b)?;
    Ok(b)
}

/// A node's static Noise identity (X25519). Secret-side only.
#[derive(Clone)]
pub struct NodeNoise {
    private: Vec<u8>,
}

impl NodeNoise {
    pub fn generate() -> Self {
        let kp = Builder::new(params()).generate_keypair().expect("noise keypair");
        NodeNoise { private: kp.private }
    }

    /// Reconstruct from a persisted 32-byte private key (e.g. embedded NVS flash).
    pub fn from_private(private: Vec<u8>) -> Self {
        NodeNoise { private }
    }

    /// Load the persisted 32-byte private key, or mint + persist one (owner-only).
    pub fn load_or_mint(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Ok(priv_) = fs::read(path) {
            if priv_.len() == 32 {
                return Ok(NodeNoise { private: priv_ });
            }
        }
        let me = Self::generate();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &me.private)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
        Ok(me)
    }

    /// Run the XX handshake as the initiator (the dialing side).
    pub fn connect(&self, s: &mut TcpStream) -> io::Result<NoiseSession> {
        let mut hs = Builder::new(params())
            .local_private_key(&self.private)
            .build_initiator()
            .map_err(noise_err)?;
        let mut buf = [0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(noise_err)?;
        write_blob(s, &buf[..n])?;
        let msg = read_blob(s)?;
        hs.read_message(&msg, &mut buf).map_err(noise_err)?;
        let n = hs.write_message(&[], &mut buf).map_err(noise_err)?;
        write_blob(s, &buf[..n])?;
        let ts = hs.into_transport_mode().map_err(noise_err)?;
        let remote_static = ts.get_remote_static().map(<[u8]>::to_vec).unwrap_or_default();
        Ok(NoiseSession { ts, remote_static })
    }

    /// Run the XX handshake as the responder (the accepting side).
    pub fn accept(&self, s: &mut TcpStream) -> io::Result<NoiseSession> {
        let mut hs = Builder::new(params())
            .local_private_key(&self.private)
            .build_responder()
            .map_err(noise_err)?;
        let mut buf = [0u8; 1024];
        let msg = read_blob(s)?;
        hs.read_message(&msg, &mut buf).map_err(noise_err)?;
        let n = hs.write_message(&[], &mut buf).map_err(noise_err)?;
        write_blob(s, &buf[..n])?;
        let msg = read_blob(s)?;
        hs.read_message(&msg, &mut buf).map_err(noise_err)?;
        let ts = hs.into_transport_mode().map_err(noise_err)?;
        let remote_static = ts.get_remote_static().map(<[u8]>::to_vec).unwrap_or_default();
        Ok(NoiseSession { ts, remote_static })
    }
}

/// An established encrypted channel. Frames are `[kind][payload]`, encrypted.
pub struct NoiseSession {
    ts: TransportState,
    /// The peer's static Noise public key (X25519), captured at handshake — the
    /// channel identity a node pins against an allowlist (C2a).
    remote_static: Vec<u8>,
}

impl NoiseSession {
    /// The peer's static Noise public key, or an empty slice if the pattern did
    /// not carry one (XX always does once the handshake completes).
    pub fn peer_static(&self) -> &[u8] {
        &self.remote_static
    }

    pub fn send(&mut self, s: &mut TcpStream, kind: u8, payload: &[u8]) -> io::Result<()> {
        if payload.len() + 1 > MAX_PLAIN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds one noise message (chunking is a follow-up)",
            ));
        }
        let mut plain = Vec::with_capacity(1 + payload.len());
        plain.push(kind);
        plain.extend_from_slice(payload);
        let mut out = vec![0u8; plain.len() + 16];
        let n = self.ts.write_message(&plain, &mut out).map_err(noise_err)?;
        write_blob(s, &out[..n])
    }

    pub fn recv(&mut self, s: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
        let cipher = read_blob(s)?;
        let mut out = vec![0u8; cipher.len()];
        let n = self.ts.read_message(&cipher, &mut out).map_err(noise_err)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "empty frame"));
        }
        Ok((out[0], out[1..n].to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn handshake_roundtrip_and_peer_key() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let server = NodeNoise::generate();
        let server_pub = {
            // derive the server's static pub by completing a handshake below
            server.clone()
        };
        let _ = server_pub;
        let h = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut sess = server.accept(&mut s).unwrap();
            let (kind, payload) = sess.recv(&mut s).unwrap();
            assert_eq!(kind, 7);
            sess.send(&mut s, 9, &payload).unwrap();
        });
        let client = NodeNoise::generate();
        let mut s = TcpStream::connect(addr).unwrap();
        let mut sess = client.connect(&mut s).unwrap();
        assert_eq!(sess.peer_static().len(), 32); // captured the responder's static key
        sess.send(&mut s, 7, b"limits to growth").unwrap();
        let (kind, payload) = sess.recv(&mut s).unwrap();
        assert_eq!(kind, 9);
        assert_eq!(payload, b"limits to growth");
        h.join().unwrap();
    }
}
