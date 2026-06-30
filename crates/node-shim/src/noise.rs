//! Authenticated, encrypted node transport (R2). Noise XX over a blocking TCP
//! stream — the same pattern as the full node, so a shim device and a hosted node
//! complete a handshake with each other.

use std::io::{self, Read, Write};
use std::net::TcpStream;

use snow::params::NoiseParams;
use snow::{Builder, TransportState};

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_BLOB: usize = 1 << 16; // 64 KiB cap before allocating (tight for embedded)
const MAX_PLAIN: usize = 65535 - 16;

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

/// A node's static Noise identity (X25519).
#[derive(Clone)]
pub struct NodeNoise {
    private: Vec<u8>,
}

impl NodeNoise {
    pub fn generate() -> Self {
        let kp = Builder::new(params()).generate_keypair().expect("noise keypair");
        NodeNoise { private: kp.private }
    }

    pub fn from_private(private: Vec<u8>) -> Self {
        NodeNoise { private }
    }

    /// Run the XX handshake as the initiator (dialing side).
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
        Ok(NoiseSession { ts: hs.into_transport_mode().map_err(noise_err)? })
    }

    /// Run the XX handshake as the responder (accepting side).
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
        Ok(NoiseSession { ts: hs.into_transport_mode().map_err(noise_err)? })
    }
}

/// An established encrypted channel. Frames are `[kind][payload]`, encrypted.
pub struct NoiseSession {
    ts: TransportState,
}

impl NoiseSession {
    pub fn send(&mut self, s: &mut TcpStream, kind: u8, payload: &[u8]) -> io::Result<()> {
        if payload.len() + 1 > MAX_PLAIN {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
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
