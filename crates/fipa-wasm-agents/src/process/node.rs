//! Cross-node transport over TCP/IP — one agent per node (a container).
//!
//! Each [`Node`] hosts a single agent, binds a TCP address (its return address),
//! and routes the agent's outbound messages to **other nodes by IP:port**. It is
//! the FIPA *Agent Communication Channel*: identity is a UUID, location is
//! resolved separately.
//!
//! ## Security (M1: R1 + R4)
//!
//! - **Signed envelope (R1).** Every [`NodeMsg`] is signed by the *sending node's*
//!   Ed25519 key over `(to, from, from_addr, unl, body, nonce, sender_pub)`. The
//!   receiver verifies before delivery, so a message cannot be **tampered in
//!   transit** and the return address cannot be silently rewritten
//!   (`THREAT_MODEL.md` C3). The signature carries `sender_pub`; binding `from` to
//!   the *authorized* node for that agent (full C1 closure) needs authenticated
//!   AMS `bind` (R3, M2) — the rails are here, the registry check lands next.
//! - **Reserved-sender rejection (C5).** A wire message whose `from` is a reserved
//!   system id (`ams`/`df`/`pa`/`llm`/`boot`/…) is dropped — no remote injection of
//!   forged platform/tool replies or kickoffs.
//! - **Frame cap + timeouts (R4).** `read_frame` refuses an oversized length
//!   *before* allocating; dials use a connect+read+write timeout.
//!
//! Local injection ([`Node::inject`]) is in-process and trusted, so it bypasses the
//! wire gate; a node kickstarts its own agent with a [`Node::sealed_kick`] — a
//! self-addressed, self-signed message — instead of an unauthenticated `boot`.
//!
//! ## Addressing
//! - **bootstrap** — well-known aliases (`ams`, `df`, `pa`) → addresses, from config;
//! - **return address** — every signed [`NodeMsg`] carries the sender's address,
//!   cached on receipt, so replies always have a route;
//! - **AMS resolution** — an unknown UUID is resolved by a synchronous `RESOLVE`
//!   request to the AMS node (unsigned control frame; authenticating it is R3/M2).

use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use crate::adapters::{self, NodeCrypto, NodeNoise};
use crate::wasm::AgentRuntime;

const KIND_MSG: u8 = 1;
const KIND_RESOLVE_REQ: u8 = 2;
const KIND_RESOLVE_RESP: u8 = 3;

/// A short dial timeout bounds connect/read/write so a slow or hostile peer cannot
/// stall a handler (R4; partial mitigation of `THREAT_MODEL.md` H3). The frame-size
/// cap now lives in the Noise transport ([`crate::adapters::noise`]).
const DIAL_TIMEOUT: Duration = Duration::from_secs(2);

/// A message in flight between nodes. `from_addr` is the sender's return address;
/// `nonce`/`sig`/`sender_pub` authenticate it (R1).
#[derive(Clone, Debug, Default)]
pub struct NodeMsg {
    pub to: String,
    pub from: String,
    pub from_addr: String,
    pub unl: Vec<u8>,
    pub body: Vec<u8>,
    /// Anti-replay nonce (16 bytes when signed).
    pub nonce: Vec<u8>,
    /// Ed25519 signature over [`signing_bytes`] (64 bytes when signed).
    pub sig: Vec<u8>,
    /// The signing node's public key (32 bytes when signed).
    pub sender_pub: Vec<u8>,
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

fn encode_msg(m: &NodeMsg) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, m.to.as_bytes());
    put(&mut b, m.from.as_bytes());
    put(&mut b, m.from_addr.as_bytes());
    put(&mut b, &m.unl);
    put(&mut b, &m.body);
    put(&mut b, &m.nonce);
    put(&mut b, &m.sig);
    put(&mut b, &m.sender_pub);
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
        nonce: get(p, &mut i)?,
        sig: get(p, &mut i)?,
        sender_pub: get(p, &mut i)?,
    })
}

/// The exact bytes covered by the signature: every field **except** `sig`.
fn signing_bytes(m: &NodeMsg) -> Vec<u8> {
    let mut b = Vec::new();
    put(&mut b, m.to.as_bytes());
    put(&mut b, m.from.as_bytes());
    put(&mut b, m.from_addr.as_bytes());
    put(&mut b, &m.unl);
    put(&mut b, &m.body);
    put(&mut b, &m.nonce);
    put(&mut b, &m.sender_pub);
    b
}

/// Dial `addr` with a bounded connect/read/write timeout (R4).
fn dial(addr: &str) -> io::Result<TcpStream> {
    let sa = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unresolvable address"))?;
    let s = TcpStream::connect_timeout(&sa, DIAL_TIMEOUT)?;
    s.set_read_timeout(Some(DIAL_TIMEOUT)).ok();
    s.set_write_timeout(Some(DIAL_TIMEOUT)).ok();
    Ok(s)
}

// ── the node ────────────────────────────────────────────────────────────

/// A node: one agent, a TCP address, a routing table, and a signing key.
pub struct Node {
    me: String,                          // this agent's UUID
    alias: String,                       // friendly name (also an accepted `to`)
    addr: String,                        // my bind address (return address)
    agent: Box<dyn AgentRuntime + Send>, // the one hosted agent
    routes: HashMap<String, String>,     // id/alias -> address (bootstrap + learned)
    ams_addr: Option<String>,            // where to RESOLVE unknown UUIDs
    sink: Option<Sender<NodeMsg>>,       // undeliverable (e.g. "result")
    key: NodeCrypto,                     // this node's Ed25519 identity (signs/verifies)
    keys: HashMap<String, [u8; 32]>,     // R3: from-uuid -> authorized node pubkey (TOFU)
    noise: NodeNoise,                    // R2: static Noise identity (encrypts the channel)
    kick_rx: Option<Receiver<(Vec<u8>, Vec<u8>)>>, // local, trusted kickoff injections
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
            key: NodeCrypto::generate(),
            keys: HashMap::new(),
            noise: NodeNoise::generate(),
            kick_rx: None,
        }
    }

    /// Use a persisted node key at `path` (mint+persist on first run) instead of an
    /// ephemeral one — so a node keeps its signing identity across restarts.
    pub fn load_key(&mut self, path: impl AsRef<std::path::Path>) -> io::Result<()> {
        self.key = NodeCrypto::load_or_mint(path)?;
        Ok(())
    }

    /// Use a persisted Noise static key at `path` (R2) so the node's channel
    /// identity is stable across restarts.
    pub fn load_noise(&mut self, path: impl AsRef<std::path::Path>) -> io::Result<()> {
        self.noise = NodeNoise::load_or_mint(path)?;
        Ok(())
    }

    /// Provide a channel of local, trusted kickoff injections `(unl, body)`. The
    /// serve loop delivers them in-process — they never touch the wire (so a
    /// kickoff needs no reserved `boot` sender, `THREAT_MODEL.md` C5).
    pub fn set_kick(&mut self, rx: Receiver<(Vec<u8>, Vec<u8>)>) {
        self.kick_rx = Some(rx);
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

    /// Stamp `sender_pub`/`nonce` and sign a message with this node's key (R1).
    fn seal(&self, m: &mut NodeMsg) {
        m.sender_pub = self.key.public_key().to_vec();
        m.nonce = self.key.nonce().to_vec();
        m.sig = Vec::new();
        m.sig = self.key.sign(&signing_bytes(m)).to_vec();
    }

    /// Inject a *local* message (trusted, in-process) — bypasses the wire gate.
    pub fn inject(&mut self, unl: &[u8], body: &[u8]) {
        self.deliver_local(NodeMsg {
            to: self.me.clone(),
            from: "boot".into(),
            unl: unl.to_vec(),
            body: body.to_vec(),
            ..Default::default()
        });
    }

    /// Serve until `shutdown`: accept connections, gate+deliver messages, answer
    /// RESOLVE requests (from the local AMS agent).
    pub fn serve(&mut self, listener: TcpListener, shutdown: Arc<AtomicBool>) {
        listener.set_nonblocking(true).ok();
        while !shutdown.load(Ordering::Relaxed) {
            // Drain local kickoff injections (trusted, in-process — never the wire).
            let mut kicks = Vec::new();
            if let Some(rx) = &self.kick_rx {
                while let Ok(k) = rx.try_recv() {
                    kicks.push(k);
                }
            }
            for (unl, body) in kicks {
                self.deliver_local(NodeMsg {
                    to: self.me.clone(),
                    from: self.me.clone(),
                    unl,
                    body,
                    ..Default::default()
                });
            }

            match listener.accept() {
                Ok((mut s, _)) => {
                    s.set_read_timeout(Some(DIAL_TIMEOUT)).ok();
                    s.set_write_timeout(Some(DIAL_TIMEOUT)).ok();
                    // R2: every connection begins with a Noise XX handshake — an
                    // un-handshaked (plaintext / unauthenticated) peer gets nowhere.
                    let Ok(mut sess) = self.noise.accept(&mut s) else { continue };
                    if let Ok((kind, payload)) = sess.recv(&mut s) {
                        match kind {
                            KIND_MSG => {
                                if let Some(m) = decode_msg(&payload) {
                                    self.accept_wire(m);
                                }
                            }
                            KIND_RESOLVE_REQ => {
                                let uuid = String::from_utf8_lossy(&payload).to_string();
                                let addr = self.resolve_local(&uuid).unwrap_or_default();
                                let _ = sess.send(&mut s, KIND_RESOLVE_RESP, addr.as_bytes());
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

    /// The wire gate (R1/C5): reject reserved senders and bad signatures, then
    /// deliver. Maximal forensic detail node-side, nothing back to the sender.
    fn accept_wire(&mut self, msg: NodeMsg) {
        if !self.wire_admit(&msg) {
            crate::flow!(
                "[{}] ⛔ dropped wire msg from '{}' (reserved or bad signature)",
                self.alias,
                msg.from
            );
            return;
        }
        if !self.authorize(&msg) {
            crate::flow!(
                "[{}] ⛔ impersonation of '{}' — sender key ≠ first-seen (TOFU)",
                self.alias,
                msg.from
            );
            return;
        }
        self.deliver_local(msg);
    }

    /// R3: trust-on-first-use from-authorization. The first node key seen signing
    /// for a given `from` uuid owns it; a later message claiming that uuid under a
    /// different key is rejected as impersonation (`THREAT_MODEL.md` C1/C2/C5).
    /// Authoritative AMS-distributed keys + owner delegation (`MOBILITY.md` §7)
    /// strengthen this in M5.
    fn authorize(&mut self, m: &NodeMsg) -> bool {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&m.sender_pub); // length already checked in wire_admit
        match self.keys.get(&m.from) {
            Some(known) => *known == pk,
            None => {
                self.keys.insert(m.from.clone(), pk);
                true
            }
        }
    }

    /// Admission check for a message arriving over the wire: not a reserved sender,
    /// and a structurally-present, valid signature.
    fn wire_admit(&self, m: &NodeMsg) -> bool {
        if adapters::is_reserved_sender(&m.from) {
            return false;
        }
        if m.sig.len() != 64 || m.sender_pub.len() != 32 {
            return false;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&m.sender_pub);
        let mut sg = [0u8; 64];
        sg.copy_from_slice(&m.sig);
        adapters::verify(&pk, &signing_bytes(m), &sg)
    }

    /// Deliver to the local agent and route its replies (no gate — caller vouches).
    fn deliver_local(&mut self, msg: NodeMsg) {
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

    /// Route one outbound message: seal (sign) it, resolve the recipient's address,
    /// and send it.
    fn emit(&mut self, to: &str, unl: &[u8], body: &[u8]) {
        let mut m = NodeMsg {
            to: to.into(),
            from: self.me.clone(),
            from_addr: self.addr.clone(),
            unl: unl.to_vec(),
            body: body.to_vec(),
            ..Default::default()
        };
        self.seal(&mut m);
        match self.address_of(to) {
            Some(addr) => {
                let _ = self.send_to(&addr, &m);
            }
            None => {
                if let Some(sink) = &self.sink {
                    let _ = sink.send(m); // e.g. "result" — surfaced, not routed
                }
            }
        }
    }

    /// Send one sealed message to `addr` over a fresh Noise channel — the R1
    /// signed envelope travels inside the R2 encrypted, mutually-authenticated link.
    fn send_to(&self, addr: &str, m: &NodeMsg) -> io::Result<()> {
        let mut s = dial(addr)?;
        let mut sess = self.noise.connect(&mut s)?;
        sess.send(&mut s, KIND_MSG, &encode_msg(m))
    }

    /// Find a recipient's address: bootstrap/cache, else ask the AMS node.
    fn address_of(&mut self, to: &str) -> Option<String> {
        if let Some(a) = self.routes.get(to) {
            return Some(a.clone());
        }
        let ams = self.ams_addr.clone()?;
        let mut s = dial(&ams).ok()?;
        let mut sess = self.noise.connect(&mut s).ok()?;
        sess.send(&mut s, KIND_RESOLVE_REQ, to.as_bytes()).ok()?;
        let (kind, payload) = sess.recv(&mut s).ok()?;
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

    fn dummy_node() -> Node {
        Node::new("N", "n", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)))
    }

    /// Build a NodeMsg from `from`, signed by an arbitrary node key `k`.
    fn signed_by(k: &NodeCrypto, from: &str) -> NodeMsg {
        let mut m = NodeMsg { to: "x".into(), from: from.into(), ..Default::default() };
        m.sender_pub = k.public_key().to_vec();
        m.nonce = k.nonce().to_vec();
        m.sig = k.sign(&signing_bytes(&m)).to_vec();
        m
    }

    #[test]
    fn wire_admit_accepts_sealed_self_message_and_rejects_tamper() {
        let n = dummy_node();
        let mut m = NodeMsg { to: "x".into(), from: "agent-uuid".into(), ..Default::default() };
        n.seal(&mut m);
        assert!(n.wire_admit(&m)); // genuine signature

        let mut t = m.clone();
        t.body = b"evil".to_vec(); // tampered after signing
        assert!(!n.wire_admit(&t));
    }

    #[test]
    fn wire_admit_rejects_reserved_senders() {
        let n = dummy_node();
        for who in ["ams", "df", "pa", "llm", "boot"] {
            let mut m = NodeMsg { to: "x".into(), from: who.into(), ..Default::default() };
            n.seal(&mut m); // even a valid signature can't launder a reserved sender
            assert!(!n.wire_admit(&m), "reserved '{who}' must be rejected");
        }
    }

    #[test]
    fn authorize_is_tofu_and_rejects_impersonation() {
        let mut n = dummy_node();
        let (k1, k2) = (NodeCrypto::generate(), NodeCrypto::generate());

        let first = signed_by(&k1, "X");
        assert!(n.wire_admit(&first) && n.authorize(&first)); // first key for X — owns it
        assert!(n.authorize(&signed_by(&k1, "X"))); // same key — still fine

        let impostor = signed_by(&k2, "X");
        assert!(n.wire_admit(&impostor)); // the signature itself is valid...
        assert!(!n.authorize(&impostor)); // ...but it's a different key for X → rejected
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
        let (ktx, krx) = mpsc::channel();
        na.set_kick(krx);
        let sda = shutdown.clone();
        let ha = thread::spawn(move || na.serve(la, sda));

        // Kick A locally; A → ping → B (over Noise) → pong → A → result(sink).
        ktx.send((b"obj(kick, x)".to_vec(), Vec::new())).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(5)).expect("A should surface a result");
        assert_eq!(String::from_utf8_lossy(&got.unl), "obj(done, x)");

        shutdown.store(true, Ordering::Relaxed);
        ha.join().ok();
        hb.join().ok();
    }
}
