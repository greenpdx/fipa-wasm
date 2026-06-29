//! Cross-node transport over TCP/IP — one agent per node (a container).
//!
//! Each [`Node`] hosts a single agent, binds a TCP address (its return address),
//! and routes the agent's outbound messages to **other nodes by IP:port**. It is
//! the FIPA *Agent Communication Channel*: identity is a UUID, location is
//! resolved separately.
//!
//! Addressing (no static map of dynamic UUIDs needed):
//! - **bootstrap** — well-known aliases (`ams`, `df`, `pa`) → addresses, from
//!   config;
//! - **return address** — every [`NodeMsg`] carries the sender's address, cached
//!   on receipt, so replies always have a route;
//! - **AMS resolution** — an unknown UUID is resolved by a synchronous
//!   `RESOLVE` request to the AMS node, which answers from its local AMS agent.
//!
//! At startup a node **registers**: `bind` its UUID→address with AMS (white
//! pages) and, if it provides a service, `offer` it to DF (yellow pages).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use crate::wasm::AgentRuntime;

const KIND_MSG: u8 = 1;
const KIND_RESOLVE_REQ: u8 = 2;
const KIND_RESOLVE_RESP: u8 = 3;

/// A message in flight between nodes. `from_addr` is the sender's return address.
#[derive(Clone, Debug, Default)]
pub struct NodeMsg {
    pub to: String,
    pub from: String,
    pub from_addr: String,
    pub unl: Vec<u8>,
    pub body: Vec<u8>,
}

// ── length-prefixed wire codec ──────────────────────────────────────────

fn put(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}
fn get(buf: &[u8], p: &mut usize) -> Option<Vec<u8>> {
    if *p + 4 > buf.len() {
        return None;
    }
    let n = u32::from_be_bytes(buf[*p..*p + 4].try_into().ok()?) as usize;
    *p += 4;
    if *p + n > buf.len() {
        return None;
    }
    let v = buf[*p..*p + n].to_vec();
    *p += n;
    Some(v)
}

fn write_frame(s: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    s.write_all(&[kind])?;
    s.write_all(&(payload.len() as u32).to_be_bytes())?;
    s.write_all(payload)?;
    s.flush()
}
fn read_frame(s: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut k = [0u8; 1];
    s.read_exact(&mut k)?;
    let mut l = [0u8; 4];
    s.read_exact(&mut l)?;
    let n = u32::from_be_bytes(l) as usize;
    let mut p = vec![0u8; n];
    s.read_exact(&mut p)?;
    Ok((k[0], p))
}

fn encode_msg(m: &NodeMsg) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, m.to.as_bytes());
    put(&mut b, m.from.as_bytes());
    put(&mut b, m.from_addr.as_bytes());
    put(&mut b, &m.unl);
    put(&mut b, &m.body);
    b
}
fn decode_msg(p: &[u8]) -> Option<NodeMsg> {
    let mut i = 0;
    Some(NodeMsg {
        to: String::from_utf8(get(p, &mut i)?).ok()?,
        from: String::from_utf8(get(p, &mut i)?).ok()?,
        from_addr: String::from_utf8(get(p, &mut i)?).ok()?,
        unl: get(p, &mut i)?,
        body: get(p, &mut i)?,
    })
}

/// Send one message to a node at `addr` (a brief connect-write-close).
pub fn send_message(addr: &str, m: &NodeMsg) -> io::Result<()> {
    let mut s = TcpStream::connect(addr)?;
    write_frame(&mut s, KIND_MSG, &encode_msg(m))
}

// ── the node ────────────────────────────────────────────────────────────

/// A node: one agent, a TCP address, and a routing table.
pub struct Node {
    me: String,                              // this agent's UUID
    alias: String,                           // friendly name (also an accepted `to`)
    addr: String,                            // my bind address (return address)
    agent: Box<dyn AgentRuntime + Send>,     // the one hosted agent
    routes: HashMap<String, String>,         // id/alias -> address (bootstrap + learned)
    ams_addr: Option<String>,                // where to RESOLVE unknown UUIDs
    sink: Option<Sender<NodeMsg>>,           // undeliverable (e.g. "result")
}

impl Node {
    pub fn new(uuid: &str, alias: &str, addr: &str, agent: Box<dyn AgentRuntime + Send>) -> Self {
        Node {
            me: uuid.into(),
            alias: alias.into(),
            addr: addr.into(),
            agent,
            routes: HashMap::new(),
            ams_addr: None,
            sink: None,
        }
    }

    /// Bootstrap: a well-known peer (alias or UUID) lives at `addr`.
    pub fn add_route(&mut self, who: &str, addr: &str) {
        self.routes.insert(who.into(), addr.into());
    }

    /// The AMS node to ask when a UUID's address is unknown.
    pub fn set_ams(&mut self, addr: &str) {
        self.ams_addr = Some(addr.into());
    }

    /// Where undeliverable messages (e.g. a buyer's `result`) are surfaced.
    pub fn set_sink(&mut self, tx: Sender<NodeMsg>) {
        self.sink = Some(tx);
    }

    /// Register with the platform: `bind` my UUID→address with AMS, and `offer`
    /// a service to DF if I provide one.
    pub fn register(&mut self, service: Option<&str>) {
        if self.routes.contains_key("ams") {
            let body = serde_json::json!({ "agent": self.me, "address": self.addr }).to_string();
            self.emit("ams", b"obj(bind, agent)", body.as_bytes());
        }
        if let Some(svc) = service {
            if self.routes.contains_key("df") {
                self.emit("df", format!("obj(offer, {svc})").as_bytes(), b"");
            }
        }
    }

    /// Inject a local message (e.g. the buyer's kickoff) as if from `boot`.
    pub fn inject(&mut self, unl: &[u8], body: &[u8]) {
        self.deliver(NodeMsg {
            to: self.me.clone(),
            from: "boot".into(),
            from_addr: String::new(),
            unl: unl.to_vec(),
            body: body.to_vec(),
        });
    }

    /// Serve until `shutdown`: accept connections, deliver messages, answer
    /// RESOLVE requests (from the local AMS agent).
    pub fn serve(&mut self, listener: TcpListener, shutdown: Arc<AtomicBool>) {
        listener.set_nonblocking(true).ok();
        while !shutdown.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    if let Ok((kind, payload)) = read_frame(&mut s) {
                        match kind {
                            KIND_MSG => {
                                if let Some(m) = decode_msg(&payload) {
                                    self.deliver(m);
                                }
                            }
                            KIND_RESOLVE_REQ => {
                                let uuid = String::from_utf8_lossy(&payload).to_string();
                                let addr = self.resolve_local(&uuid).unwrap_or_default();
                                let _ = write_frame(&mut s, KIND_RESOLVE_RESP, addr.as_bytes());
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
    }

    /// Deliver to the local agent and route its replies.
    fn deliver(&mut self, msg: NodeMsg) {
        crate::flow!("[{}] ← {} : {}", self.alias, msg.from, String::from_utf8_lossy(&msg.unl));
        // Cache the sender's return address so replies have a route.
        if !msg.from.is_empty() && !msg.from_addr.is_empty() {
            self.routes.insert(msg.from.clone(), msg.from_addr.clone());
        }
        let _ = self.agent.config(&msg.from, &msg.unl, &msg.body);
        for s in self.agent.take_sends() {
            self.emit(&s.receiver, &s.unl, &s.body);
        }
    }

    /// Route one outbound message: resolve the recipient's address and send it.
    fn emit(&mut self, to: &str, unl: &[u8], body: &[u8]) {
        let m = NodeMsg {
            to: to.into(),
            from: self.me.clone(),
            from_addr: self.addr.clone(),
            unl: unl.to_vec(),
            body: body.to_vec(),
        };
        match self.address_of(to) {
            Some(addr) => {
                let _ = send_message(&addr, &m);
            }
            None => {
                if let Some(sink) = &self.sink {
                    let _ = sink.send(m); // e.g. "result" — surfaced, not routed
                }
            }
        }
    }

    /// Find a recipient's address: bootstrap/cache, else ask the AMS node.
    fn address_of(&mut self, to: &str) -> Option<String> {
        if let Some(a) = self.routes.get(to) {
            return Some(a.clone());
        }
        let ams = self.ams_addr.clone()?;
        let mut s = TcpStream::connect(&ams).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok();
        write_frame(&mut s, KIND_RESOLVE_REQ, to.as_bytes()).ok()?;
        let (kind, payload) = read_frame(&mut s).ok()?;
        if kind != KIND_RESOLVE_RESP {
            return None;
        }
        let addr = String::from_utf8(payload).ok()?;
        if addr.is_empty() {
            return None;
        }
        self.routes.insert(to.into(), addr.clone()); // cache
        Some(addr)
    }

    /// Answer a RESOLVE by asking the local agent to `locate` the UUID. Only the
    /// AMS agent returns an address; others produce nothing.
    fn resolve_local(&mut self, uuid: &str) -> Option<String> {
        let body = serde_json::json!({ "agent": uuid }).to_string();
        self.agent.config("resolver", b"obj(locate, agent)", body.as_bytes()).ok()?;
        let reply = self.agent.take_sends().into_iter().next()?;
        let v: serde_json::Value = serde_json::from_slice(&reply.body).ok()?;
        v.get("address")?.as_str().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::NativeRuntime;
    use std::sync::mpsc;
    use std::thread;
    use unl_agent::{Agent, Ctx};

    struct Pinger {
        target: String,
    }
    impl Agent for Pinger {
        fn on_message(&mut self, unl: &str, _b: &[u8], ctx: &mut Ctx) {
            if unl.contains("kick") {
                ctx.send(&self.target, "obj(ping, x)", Vec::new());
            } else if unl.contains("pong") {
                ctx.send("result", "obj(done, x)", Vec::new());
            }
        }
    }
    struct Ponger;
    impl Agent for Ponger {
        fn on_message(&mut self, unl: &str, _b: &[u8], ctx: &mut Ctx) {
            if unl.contains("ping") {
                let from = ctx.from().to_string(); // reply via the authenticated sender
                ctx.send(from, "obj(pong, x)", Vec::new());
            }
        }
    }

    #[test]
    fn two_nodes_exchange_over_tcp() {
        let la = TcpListener::bind("127.0.0.1:0").unwrap();
        let aa = la.local_addr().unwrap().to_string();
        let lb = TcpListener::bind("127.0.0.1:0").unwrap();
        let bb = lb.local_addr().unwrap().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        // Node B (ponger): knows nothing — it replies via the cached return address.
        let mut nb = Node::new("B", "b", &bb, Box::new(NativeRuntime::new(Ponger)));
        let sdb = shutdown.clone();
        let hb = thread::spawn(move || nb.serve(lb, sdb));

        // Node A (pinger → "b"): bootstrap route b -> B's address; sink for "result".
        let mut na = Node::new("A", "a", &aa, Box::new(NativeRuntime::new(Pinger { target: "b".into() })));
        na.add_route("b", &bb);
        na.set_sink(tx);
        let sda = shutdown.clone();
        let ha = thread::spawn(move || na.serve(la, sda));

        // Kick A over TCP; A → ping → B → pong → A → result(sink).
        send_message(&aa, &NodeMsg { to: "A".into(), from: "boot".into(), unl: b"obj(kick, x)".to_vec(), ..Default::default() }).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(5)).expect("A should surface a result");
        assert_eq!(String::from_utf8_lossy(&got.unl), "obj(done, x)");

        shutdown.store(true, Ordering::Relaxed);
        ha.join().ok();
        hb.join().ok();
    }
}
