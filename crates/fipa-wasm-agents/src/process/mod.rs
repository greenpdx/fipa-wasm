//! Process-per-agent isolation (the "host" profile).
//!
//! An agent runs in its own `agent-host` child process, reached over a Unix
//! domain socket. The child applies `setrlimit` (RAM/CPU caps) before loading
//! the agent, so a runaway or memory-hungry agent is bounded by the OS and can
//! be SIGKILLed without touching the node. [`ProcessRuntime`] is the node-side
//! handle and implements [`AgentRuntime`], so the rest of the node drives an
//! isolated agent exactly like an in-process one.
//!
//! The wire framing here is deliberately the same shape as a message
//! (`receiver`, `unl`, `body`), so the UDS link doubles as the cross-node
//! transport later: an isolated agent is "a remote agent over a local socket".

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::wasm::{AgentRuntime, OutboundIntent};

mod agents;
mod manage;
mod migrate;
mod node;
mod resolve;
mod router;
pub use agents::native_agent;
pub use manage::{build_runtime, build_wasm, ManagedAgent, Profile, Recipe};
pub use migrate::{AgentSnapshot, Handoff, MigratePayload};
pub use node::{Node, NodeMsg};
pub use resolve::{resolve, Resolution};
pub use router::{Envelope, Router};

/// Resource caps applied by the child before it loads the agent. `0` = leave
/// the limit unchanged.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Address-space cap in bytes (RLIMIT_AS).
    pub mem_bytes: u64,
    /// CPU-seconds cap (RLIMIT_CPU).
    pub cpu_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { mem_bytes: 256 * 1024 * 1024, cpu_secs: 30 }
    }
}

/// Which agent the child should host.
#[derive(Clone, Debug)]
pub enum AgentSpec {
    /// Load a wasm agent bundle from a file.
    Wasm(PathBuf),
    /// Run a native agent compiled into the agent-host, by name.
    Native(String),
}

// ─────────────────────────────  wire frames  ─────────────────────────────

/// One IPC frame between the node and an agent-host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// node → host: run the agent's init.
    Init,
    /// node → host: deliver `(unl, body)` from sender `from`.
    Config { from: String, unl: Vec<u8>, body: Vec<u8> },
    /// host → node: the agent emitted a message.
    Emit { receiver: String, unl: Vec<u8>, body: Vec<u8> },
    /// host → node: end of the response to the last Init/Config.
    Done,
}

fn put(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}

fn get(buf: &[u8], p: &mut usize) -> io::Result<Vec<u8>> {
    let end = p.checked_add(4).filter(|e| *e <= buf.len());
    let Some(e) = end else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short frame"));
    };
    let n = u32::from_be_bytes(buf[*p..e].try_into().unwrap()) as usize;
    *p = e;
    let end = p.checked_add(n).filter(|e| *e <= buf.len());
    let Some(e) = end else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short frame"));
    };
    let v = buf[*p..e].to_vec();
    *p = e;
    Ok(v)
}

/// Write a length-prefixed frame.
pub fn write_frame<W: Write>(w: &mut W, frame: &Frame) -> io::Result<()> {
    let mut buf = Vec::new();
    match frame {
        Frame::Init => buf.push(0x01),
        Frame::Config { from, unl, body } => {
            buf.push(0x02);
            put(&mut buf, from.as_bytes());
            put(&mut buf, unl);
            put(&mut buf, body);
        }
        Frame::Emit { receiver, unl, body } => {
            buf.push(0x10);
            put(&mut buf, receiver.as_bytes());
            put(&mut buf, unl);
            put(&mut buf, body);
        }
        Frame::Done => buf.push(0x11),
    }
    w.write_all(&(buf.len() as u32).to_be_bytes())?;
    w.write_all(&buf)?;
    w.flush()
}

/// Read one length-prefixed frame; `Ok(None)` on a clean EOF (peer closed).
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "empty/bad frame");
    let tag = *buf.first().ok_or_else(bad)?;
    let mut p = 1;
    let frame = match tag {
        0x01 => Frame::Init,
        0x02 => Frame::Config {
            from: String::from_utf8_lossy(&get(&buf, &mut p)?).into_owned(),
            unl: get(&buf, &mut p)?,
            body: get(&buf, &mut p)?,
        },
        0x10 => Frame::Emit {
            receiver: String::from_utf8_lossy(&get(&buf, &mut p)?).into_owned(),
            unl: get(&buf, &mut p)?,
            body: get(&buf, &mut p)?,
        },
        0x11 => Frame::Done,
        _ => return Err(bad()),
    };
    Ok(Some(frame))
}

// ───────────────────────────  node side  ───────────────────────────

/// Node-side handle to an agent running in its own process. Drives it over UDS
/// and SIGKILLs the child on drop.
pub struct ProcessRuntime {
    child: Child,
    stream: UnixStream,
    sock_path: PathBuf,
    outbox: Vec<OutboundIntent>,
}

impl ProcessRuntime {
    /// Spawn an `agent-host` child for `spec`, apply `limits`, and connect.
    pub fn spawn(
        host_bin: &Path,
        spec: &AgentSpec,
        limits: &Limits,
        timeout: Duration,
    ) -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let sock_path =
            std::env::temp_dir().join(format!("fipa-agent-{}-{n}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock_path);
        let listener = UnixListener::bind(&sock_path)?;

        let mut cmd = Command::new(host_bin);
        cmd.arg("--socket").arg(&sock_path)
            .arg("--mem-bytes").arg(limits.mem_bytes.to_string())
            .arg("--cpu-secs").arg(limits.cpu_secs.to_string());
        match spec {
            AgentSpec::Wasm(p) => {
                cmd.arg("--wasm").arg(p);
            }
            AgentSpec::Native(n) => {
                cmd.arg("--native").arg(n);
            }
        }
        let child = cmd.spawn()?;

        // The child connects on startup. Accept within the timeout.
        listener.set_nonblocking(true)?;
        let deadline = std::time::Instant::now() + timeout;
        let stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err(anyhow!("agent host did not connect within {timeout:?}"));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => return Err(e.into()),
            }
        };
        stream.set_read_timeout(Some(timeout))?;
        Ok(ProcessRuntime { child, stream, sock_path, outbox: Vec::new() })
    }

    fn round_trip(&mut self, req: Frame) -> Result<()> {
        write_frame(&mut self.stream, &req)?;
        loop {
            match read_frame(&mut self.stream)? {
                Some(Frame::Emit { receiver, unl, body }) => {
                    self.outbox.push(OutboundIntent { receiver, unl, body })
                }
                Some(Frame::Done) => return Ok(()),
                Some(other) => return Err(anyhow!("unexpected frame from agent host: {other:?}")),
                None => return Err(anyhow!("agent host disconnected")),
            }
        }
    }
}

impl AgentRuntime for ProcessRuntime {
    fn init(&mut self) -> Result<()> {
        self.round_trip(Frame::Init)
    }

    fn config(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<()> {
        self.round_trip(Frame::Config {
            from: from.to_string(),
            unl: unl.to_vec(),
            body: body.to_vec(),
        })
    }

    fn take_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut self.outbox)
    }
}

impl Drop for ProcessRuntime {
    fn drop(&mut self) {
        // SIGKILL: hard, unconditional — works against a runaway loop too.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

// ───────────────────────────  child side  ───────────────────────────

/// The agent-host loop: drive `runtime` from frames on `stream` until the node
/// disconnects. Used by the `agent-host` binary.
pub fn serve(mut runtime: Box<dyn AgentRuntime>, mut stream: UnixStream) -> Result<()> {
    while let Some(frame) = read_frame(&mut stream)? {
        match frame {
            Frame::Init => {
                runtime.init()?;
                flush(runtime.as_mut(), &mut stream)?;
            }
            Frame::Config { from, unl, body } => {
                runtime.config(&from, &unl, &body)?;
                flush(runtime.as_mut(), &mut stream)?;
            }
            _ => break,
        }
    }
    Ok(())
}

fn flush(runtime: &mut dyn AgentRuntime, stream: &mut UnixStream) -> Result<()> {
    for s in runtime.take_sends() {
        write_frame(stream, &Frame::Emit { receiver: s.receiver, unl: s.unl, body: s.body })?;
    }
    write_frame(stream, &Frame::Done)?;
    Ok(())
}

/// Apply resource limits to the current process (call before loading the agent).
#[cfg(unix)]
pub fn apply_limits(limits: &Limits) {
    fn set(resource: libc::__rlimit_resource_t, value: u64) {
        if value == 0 {
            return;
        }
        let lim = libc::rlimit { rlim_cur: value, rlim_max: value };
        unsafe {
            libc::setrlimit(resource, &lim);
        }
    }
    set(libc::RLIMIT_AS, limits.mem_bytes);
    set(libc::RLIMIT_CPU, limits.cpu_secs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let frames = [
            Frame::Init,
            Frame::Config { from: "alice".into(), unl: b"agt(x, y)".to_vec(), body: b"data".to_vec() },
            Frame::Emit { receiver: "bob".into(), unl: b"agt(p, q)".to_vec(), body: vec![1, 2, 3] },
            Frame::Done,
        ];
        let mut buf = Vec::new();
        for f in &frames {
            write_frame(&mut buf, f).unwrap();
        }
        let mut cur = std::io::Cursor::new(buf);
        for f in &frames {
            assert_eq!(read_frame(&mut cur).unwrap().as_ref(), Some(f));
        }
        assert_eq!(read_frame(&mut cur).unwrap(), None); // clean EOF
    }
}
