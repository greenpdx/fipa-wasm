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

use crate::adapters::{self, NodeCrypto, NodeNoise, NoiseSession, SledStore, StateStore};
use crate::manifest::{Capability, Grant, Manifest, NodeProfile};
use crate::wasm::{AgentRuntime, OutboundIntent, WasmRuntime};
use rand::RngCore;
use std::collections::HashSet;
use unl_agent::{InferReq, SpawnReq, TimerOp};

use super::migrate::{AgentSnapshot, Handoff, MigratePayload};

const KIND_MSG: u8 = 1;
const KIND_RESOLVE_REQ: u8 = 2;
const KIND_RESOLVE_RESP: u8 = 3;
const KIND_MIGRATE: u8 = 4;

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

/// A namespaced state handle: an agent's [`unl_agent::Kv`] confined to its own
/// namespace (R8 — keys cannot escape) and bounded by a byte quota (M4).
struct ScopedKv {
    store: Arc<SledStore>,
    ns: String,
    used: Arc<std::sync::atomic::AtomicU64>,
    quota: u64,
}
impl unl_agent::Kv for ScopedKv {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.get(&self.ns, key).ok().flatten()
    }
    fn put(&self, key: &str, val: &[u8]) {
        use std::sync::atomic::Ordering::Relaxed;
        let old = self.store.get(&self.ns, key).ok().flatten().map(|v| v.len() as u64).unwrap_or(0);
        let projected = self.used.load(Relaxed).saturating_sub(old).saturating_add(val.len() as u64);
        if projected > self.quota {
            return; // quota exceeded → silent denial (the agent sees no write)
        }
        if self.store.put(&self.ns, key, val).is_ok() {
            self.used.store(projected, Relaxed);
        }
    }
    fn del(&self, key: &str) {
        use std::sync::atomic::Ordering::Relaxed;
        let old = self.store.get(&self.ns, key).ok().flatten().map(|v| v.len() as u64).unwrap_or(0);
        if self.store.del(&self.ns, key).is_ok() {
            self.used.store(self.used.load(Relaxed).saturating_sub(old), Relaxed);
        }
    }
}

/// Domain tag separating agent-app signatures from the node's own envelope /
/// migration signatures, so the signing oracle cannot be a confused deputy.
const APP_DOMAIN: &[u8] = b"fipa:agent-app:v1\0";

/// The crypto keyring handed to an agent with the `crypto` capability (M5): the
/// node's Ed25519 key, used only behind a fixed application domain.
struct NodeKeyring {
    key: NodeCrypto,
}
impl unl_agent::Keyring for NodeKeyring {
    fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        let mut buf = APP_DOMAIN.to_vec();
        buf.extend_from_slice(bytes);
        self.key.sign(&buf).to_vec()
    }
    fn verify(&self, pubkey: &[u8], bytes: &[u8], sig: &[u8]) -> bool {
        if pubkey.len() != 32 || sig.len() != 64 {
            return false;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(pubkey);
        let mut sg = [0u8; 64];
        sg.copy_from_slice(sig);
        let mut buf = APP_DOMAIN.to_vec();
        buf.extend_from_slice(bytes);
        adapters::verify(&pk, &buf, &sg)
    }
    fn public_key(&self) -> Vec<u8> {
        self.key.public_key().to_vec()
    }
    fn random(&self, n: usize) -> Vec<u8> {
        let mut v = vec![0u8; n];
        rand::rng().fill_bytes(&mut v);
        v
    }
}

/// A node-side inference backend — the `llm` capability runs the model on the
/// agent's behalf (keys/cost/model stay node-side, `AGENT_HOST_ABI.md` §7.1). Sync;
/// the node runs it off the main thread and delivers the result by message.
pub trait LlmBackend: Send + Sync {
    fn infer(&self, prompt: &str) -> Result<String, String>;
}

/// A node-side forensic audit event (M6) — **log-rich** while the agent gets only a
/// uniform denial (`AGENT_HOST_ABI.md` §11). Node-attributed and agent-unspoofable.
#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub agent: String,
    pub kind: String,
    pub detail: String,
}

/// Where audit events are recorded (a SIEM, a log, a test recorder).
pub trait AuditSink: Send + Sync {
    fn record(&self, event: &AuditEvent);
}

/// Wall-clock milliseconds since the Unix epoch (the scheduler's clock, M3).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Handle one accepted connection in its own thread (so a slow or hostile peer
/// cannot stall the accept loop or the single-threaded agent executor, H3/R7):
/// run the Noise handshake, read one frame, and hand it to the node's main loop —
/// a decoded message via `in_tx`, or a RESOLVE request via `rz_tx` (answered by
/// the main loop, which owns the agents).
fn handle_conn(
    mut s: TcpStream,
    noise: &NodeNoise,
    in_tx: &Sender<NodeMsg>,
    rz_tx: &Sender<(String, Sender<String>)>,
    mg_tx: &Sender<Vec<u8>>,
) {
    s.set_read_timeout(Some(DIAL_TIMEOUT)).ok(); // bound the handshake
    s.set_write_timeout(Some(DIAL_TIMEOUT)).ok();
    let Ok(mut sess) = noise.accept(&mut s) else { return };
    // After the handshake a KIND_MSG channel is persistent, so tolerate idle gaps
    // between messages; a long idle just recycles the connection (sender re-dials).
    s.set_read_timeout(Some(Duration::from_secs(60))).ok();
    loop {
        let (kind, payload) = match sess.recv(&mut s) {
            Ok(v) => v,
            Err(_) => return, // idle timeout / EOF / error → close
        };
        match kind {
            KIND_MSG => {
                if let Some(m) = decode_msg(&payload) {
                    let _ = in_tx.send(m);
                }
                // keep reading — the channel is persistent
            }
            KIND_MIGRATE => {
                let _ = mg_tx.send(payload); // hand the move payload to the main loop
                return; // one-shot
            }
            KIND_RESOLVE_REQ => {
                let uuid = String::from_utf8_lossy(&payload).to_string();
                let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                if rz_tx.send((uuid, resp_tx)).is_ok() {
                    if let Ok(addr) = resp_rx.recv_timeout(DIAL_TIMEOUT) {
                        let _ = sess.send(&mut s, KIND_RESOLVE_RESP, addr.as_bytes());
                    }
                }
                return; // one-shot
            }
            _ => return,
        }
    }
}

// ── the node ────────────────────────────────────────────────────────────

/// One mounted agent: its identity, friendly alias, runtime, and offered service.
struct Mounted {
    uuid: String,
    alias: String,
    runtime: Box<dyn AgentRuntime + Send>,
    service: Option<String>,
    code: Option<Vec<u8>>, // wasm bytes for a mobile agent; None for native templates
    epoch: u64,            // this agent's location epoch (R6)
    grant: Grant,          // M2: the agent's effective capability authority
}

/// A node: one **or more** local agents, a TCP address, a routing table, and the
/// node's signing + Noise identities. Co-located agents exchange messages through
/// an in-process work queue (the executor); only cross-node hops touch the wire.
pub struct Node {
    addr: String,                        // my bind address (return address)
    label: String,                       // node label for logs (the primary alias)
    primary: String,                     // first-mounted agent uuid (kick/inject target)
    agents: HashMap<String, Mounted>,    // uuid -> mounted agent
    aliases: HashMap<String, String>,    // local alias -> uuid
    routes: HashMap<String, String>,     // remote id/alias -> address (bootstrap + learned)
    ams_addr: Option<String>,            // where to RESOLVE unknown UUIDs
    sink: Option<Sender<NodeMsg>>,       // undeliverable (e.g. "result")
    key: NodeCrypto,                     // this node's Ed25519 identity (signs/verifies)
    keys: HashMap<String, [u8; 32]>,     // R3: from-uuid -> authorized node pubkey (TOFU)
    noise: NodeNoise,                    // R2: static Noise identity (encrypts the channel)
    kick_rx: Option<Receiver<(Vec<u8>, Vec<u8>)>>, // local, trusted kickoff injections
    seen: HashMap<String, u64>,          // migration replay guard: uuid -> last epoch
    profile: NodeProfile,                // M2: which capabilities this node offers
    timers: HashMap<String, HashMap<u64, u64>>, // M3: uuid -> timer_id -> deadline_ms
    store: Option<Arc<SledStore>>,       // M4: durable state backend (state capability)
    llm: Option<Arc<dyn LlmBackend>>,    // M5: inference backend (llm capability)
    pending_infers: Vec<(String, u64, String)>, // M5: (agent, req_id, prompt) to run
    audit: Option<Arc<dyn AuditSink>>,   // M6: forensic event sink (log rich)
    faults: HashMap<String, u32>,        // M6: consecutive fault count per agent
    quarantined: HashSet<String>,        // M6: agents stopped after repeated faults
    conns: HashMap<String, (TcpStream, NoiseSession)>, // persistent KIND_MSG channels per peer
}

impl Node {
    pub fn new(uuid: &str, alias: &str, addr: &str, agent: Box<dyn AgentRuntime + Send>) -> Self {
        let mut node = Node {
            addr: addr.into(),
            label: alias.into(),
            primary: uuid.into(),
            agents: HashMap::new(),
            aliases: HashMap::new(),
            routes: HashMap::new(),
            ams_addr: None,
            sink: None,
            key: NodeCrypto::generate(),
            keys: HashMap::new(),
            noise: NodeNoise::generate(),
            kick_rx: None,
            seen: HashMap::new(),
            profile: NodeProfile::normal(),
            timers: HashMap::new(),
            store: None,
            llm: None,
            pending_infers: Vec::new(),
            audit: None,
            faults: HashMap::new(),
            quarantined: HashSet::new(),
            conns: HashMap::new(),
        };
        node.mount(uuid, alias, agent, None);
        node
    }

    /// Mount an additional agent (a multi-agent / "platform" node). The agent from
    /// [`Node::new`] is the `primary`; `mount` co-locates more. Co-located agents
    /// then message each other in-process, never over the wire.
    pub fn mount(
        &mut self,
        uuid: &str,
        alias: &str,
        agent: Box<dyn AgentRuntime + Send>,
        service: Option<&str>,
    ) {
        self.aliases.insert(alias.into(), uuid.into());
        self.agents.insert(
            uuid.into(),
            Mounted {
                uuid: uuid.into(),
                alias: alias.into(),
                runtime: agent,
                service: service.map(Into::into),
                code: None,
                epoch: 0,
                grant: Grant::full(), // native = trusted infra template
            },
        );
        self.provision_state(uuid);
        self.provision_crypto(uuid);
    }

    /// Set this node's profile (e.g. `NodeProfile::iot()`), which determines the
    /// capabilities and budget ceilings a mounted agent's manifest must fit.
    pub fn set_profile(&mut self, profile: NodeProfile) {
        self.profile = profile;
    }

    /// Whether `uuid`'s agent holds `cap` — the gate every host-call consults
    /// (M2). A `false` is the uniform, opaque `denied` at runtime.
    pub fn granted(&self, uuid: &str, cap: Capability) -> bool {
        self.agents.get(uuid).map(|m| m.grant.granted(cap)).unwrap_or(false)
    }

    /// Provide a durable state store for agents that hold the `state` capability
    /// (re-provisions any already-mounted agents, e.g. the primary from `new`).
    pub fn set_store(&mut self, store: SledStore) {
        self.store = Some(Arc::new(store));
        let uuids: Vec<String> = self.agents.keys().cloned().collect();
        for u in uuids {
            self.provision_state(&u);
        }
    }

    /// If `uuid` holds the `State` capability and the node has a store, hand the
    /// agent a namespace-confined Kv handle (M4).
    fn provision_state(&mut self, uuid: &str) {
        if !self.granted(uuid, Capability::State) {
            return;
        }
        let Some(store) = self.store.clone() else { return };
        let quota = self.agents.get(uuid).map(|m| m.grant.budget.state_kb.saturating_mul(1024)).unwrap_or(0);
        let kv = Arc::new(ScopedKv {
            store,
            ns: uuid.to_string(),
            used: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            quota,
        });
        if let Some(m) = self.agents.get_mut(uuid) {
            m.runtime.set_state(kv);
        }
    }

    /// Provide an inference backend for agents that hold the `llm` capability (M5).
    pub fn set_llm(&mut self, backend: Arc<dyn LlmBackend>) {
        self.llm = Some(backend);
    }

    /// If `uuid` holds the `Crypto` capability, hand the agent a domain-separated
    /// keyring backed by the node key (M5).
    fn provision_crypto(&mut self, uuid: &str) {
        if !self.granted(uuid, Capability::Crypto) {
            return;
        }
        let kr = Arc::new(NodeKeyring { key: self.key.clone() });
        if let Some(m) = self.agents.get_mut(uuid) {
            m.runtime.set_keyring(kr);
        }
    }

    /// Queue an agent's inference requests (M5), gated by the `Llm` capability; the
    /// serve loop runs them off-thread and delivers each result as a message.
    fn apply_infer_reqs(&mut self, uuid: &str, reqs: Vec<InferReq>) {
        if reqs.is_empty() {
            return;
        }
        if !self.granted(uuid, Capability::Llm) {
            crate::flow!("[{}] ⛔ infer denied for '{}' (no Llm grant)", self.label, uuid);
            return;
        }
        for r in reqs {
            self.pending_infers.push((uuid.to_string(), r.req_id, r.prompt));
        }
    }

    /// Set a forensic audit sink (M6). Events are recorded node-side; the agent
    /// always gets only the uniform denial.
    pub fn set_audit(&mut self, sink: Arc<dyn AuditSink>) {
        self.audit = Some(sink);
    }

    /// Record an audit event (no-op without a sink).
    fn audit(&self, agent: &str, kind: &str, detail: &str) {
        if let Some(sink) = &self.audit {
            sink.record(&AuditEvent { agent: agent.into(), kind: kind.into(), detail: detail.into() });
        }
    }

    /// Supervisor (M6): track per-agent faults and quarantine an agent that faults
    /// repeatedly, so a misbehaving agent is isolated, not the node.
    fn supervise(&mut self, uuid: &str, result: &anyhow::Result<()>) {
        const MAX_FAULTS: u32 = 3;
        match result {
            Ok(()) => {
                self.faults.remove(uuid);
            }
            Err(e) => {
                let count = {
                    let n = self.faults.entry(uuid.to_string()).or_default();
                    *n += 1;
                    *n
                };
                self.audit(uuid, "fault", &e.to_string());
                if count >= MAX_FAULTS {
                    self.quarantined.insert(uuid.to_string());
                    self.audit(uuid, "quarantined", "fault threshold exceeded");
                    crate::flow!("[{}] ⛔ quarantined '{}' after {} faults", self.label, uuid, count);
                }
            }
        }
    }

    /// Spawn child agents (M6), gated by the parent's `Spawn` capability. Each child
    /// is mounted with its grants intersected with the parent's (child caps ⊆ parent).
    fn apply_spawn_reqs(&mut self, parent: &str, reqs: Vec<SpawnReq>) {
        if reqs.is_empty() {
            return;
        }
        if !self.granted(parent, Capability::Spawn) {
            self.audit(parent, "denied:spawn", "no Spawn grant");
            return;
        }
        let parent_caps = self.agents.get(parent).map(|m| m.grant.caps.clone()).unwrap_or_default();
        for r in reqs {
            let Some(mut manifest) = Manifest::from_json(&r.manifest_json) else {
                self.audit(parent, "spawn:bad-manifest", &r.uuid);
                continue;
            };
            manifest.grants.retain(|c| parent_caps.contains(c)); // child caps ⊆ parent
            match self.mount_wasm(&r.uuid, &r.alias, r.code, &manifest, None) {
                Ok(()) => self.audit(parent, "spawned", &r.uuid),
                Err(e) => self.audit(parent, "spawn:failed", &e.to_string()),
            }
        }
    }

    /// Out-gate net-scope (M4): an agent with `net = "none"` may message only
    /// co-located agents; `platform`/`any`/unset may reach the network (the node
    /// resolves the address). Finer scoping (`node:<id>`) is a future refinement.
    fn net_allows(&self, uuid: &str, to: &str) -> bool {
        match self.agents.get(uuid).map(|m| m.grant.budget.net.as_str()) {
            Some("none") => self.local_uuid(to).is_some(),
            _ => true,
        }
    }

    /// Mount a **mobile wasm agent** from its module bytes + `manifest` (HEAD) — only
    /// wasm agents move (native agents are stationary, host-instantiated templates).
    /// The manifest is fit against the node profile (M2 load-time gate); on success
    /// the effective [`Grant`] is stored and the wasm engine caps are derived from
    /// the budget. The code is retained so the agent can later be migrated.
    pub fn mount_wasm(
        &mut self,
        uuid: &str,
        alias: &str,
        code: Vec<u8>,
        manifest: &Manifest,
        service: Option<&str>,
    ) -> anyhow::Result<()> {
        let grant = self
            .profile
            .fit(manifest)
            .map_err(|e| anyhow::anyhow!("manifest does not fit node profile: {e:?}"))?;
        let caps = crate::proto::AgentCapabilities {
            max_memory_bytes: grant.budget.mem_kb.saturating_mul(1024),
            max_execution_time_ms: (grant.budget.fuel / 1_000_000).max(1),
            storage_quota_bytes: grant.budget.state_kb.saturating_mul(1024),
            ..Default::default()
        };
        let mut rt = WasmRuntime::new(&code, &caps)?;
        rt.call_init()?;
        self.aliases.insert(alias.into(), uuid.into());
        self.agents.insert(
            uuid.into(),
            Mounted {
                uuid: uuid.into(),
                alias: alias.into(),
                runtime: Box::new(rt),
                service: service.map(Into::into),
                code: Some(code),
                epoch: 0,
                grant,
            },
        );
        self.provision_state(uuid);
        self.provision_crypto(uuid);
        Ok(())
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

    /// Register with the platform: `bind` each local agent's UUID→address with AMS,
    /// and `offer` each agent's service to DF. For back-compat, `service` overrides
    /// the **primary** agent's service (single-agent nodes set it here).
    pub fn register(&mut self, service: Option<&str>) {
        if let Some(svc) = service {
            let primary = self.primary.clone();
            if let Some(m) = self.agents.get_mut(&primary) {
                m.service = Some(svc.into());
            }
        }
        let mounts: Vec<(String, Option<String>)> =
            self.agents.values().map(|m| (m.uuid.clone(), m.service.clone())).collect();
        let (have_ams, have_df) = (self.routes.contains_key("ams"), self.routes.contains_key("df"));
        for (uuid, svc) in mounts {
            if have_ams {
                let body = serde_json::json!({ "agent": uuid, "address": self.addr }).to_string();
                self.send_as(&uuid, "ams", b"obj(bind, agent)", body.as_bytes());
            }
            if let (Some(svc), true) = (svc, have_df) {
                self.send_as(&uuid, "df", format!("obj(offer, {svc})").as_bytes(), b"");
            }
        }
    }

    /// This node's Ed25519 public key (its signing identity), e.g. the handoff
    /// target a source node must authorize before migrating an agent here.
    pub fn node_pub(&self) -> [u8; 32] {
        self.key.public_key()
    }

    /// Build the signed move payload (snapshot of code+state at epoch+1, plus a
    /// handoff authorizing `dest_pub`) for a mobile agent — without sending it.
    pub fn build_migrate_payload(&mut self, uuid: &str, dest_pub: &[u8]) -> Option<Vec<u8>> {
        let m = self.agents.get_mut(uuid)?;
        let code = m.code.clone()?; // only wasm (mobile) agents have code
        let epoch = m.epoch + 1;
        let state = m.runtime.snapshot();
        let snapshot = AgentSnapshot::sealed(uuid, epoch, code, state, &self.key);
        let handoff = Handoff::sealed(uuid, dest_pub.to_vec(), epoch, &self.key);
        Some(MigratePayload { snapshot, handoff }.encode())
    }

    /// Migrate a mobile (wasm) agent to `dest_addr`, authorizing `dest_pub` to act
    /// for it: send the signed snapshot + handoff over Noise, then tombstone the
    /// local copy. The epoch arbiter (R6) + handoff prevent forking (H1); full
    /// two-phase crash-safety is the remaining hardening.
    pub fn migrate(&mut self, uuid: &str, dest_addr: &str, dest_pub: &[u8]) -> io::Result<()> {
        let payload = self
            .build_migrate_payload(uuid, dest_pub)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "agent not mobile / absent"))?;
        let mut s = dial(dest_addr)?;
        let mut sess = self.noise.connect(&mut s)?;
        sess.send(&mut s, KIND_MIGRATE, &payload)?;
        if let Some(m) = self.agents.remove(uuid) {
            self.aliases.remove(&m.alias);
        }
        Ok(())
    }

    /// Receive a migrated agent: verify the snapshot + handoff, confirm it is for
    /// this node, guard against replay (epoch must advance), instantiate the wasm
    /// from the carried code, restore state, mount it, and re-bind at AMS carrying
    /// the handoff so the AMS node can move the agent's authorized key.
    fn process_migrate(&mut self, payload: &[u8]) {
        let Some(mp) = MigratePayload::decode(payload) else { return };
        let (snap, ho) = (mp.snapshot, mp.handoff);
        if !snap.verify() || !ho.verify() {
            crate::flow!("[{}] ⛔ migrate: bad snapshot/handoff signature", self.label);
            return;
        }
        if ho.to_pub != self.key.public_key()
            || ho.agent != snap.uuid
            || ho.epoch != snap.epoch
            || ho.from_pub != snap.origin_pub
        {
            crate::flow!("[{}] ⛔ migrate: handoff not for me / inconsistent", self.label);
            return;
        }
        if self.seen.get(&snap.uuid).is_some_and(|&e| snap.epoch <= e) {
            crate::flow!("[{}] ⛔ migrate: replayed epoch {} for '{}'", self.label, snap.epoch, snap.uuid);
            return;
        }
        let caps = crate::proto::AgentCapabilities::default();
        let mut rt = match WasmRuntime::new(&snap.code, &caps) {
            Ok(rt) => rt,
            Err(_) => {
                crate::flow!("[{}] ⛔ migrate: code won't instantiate", self.label);
                return;
            }
        };
        let _ = rt.call_init();
        rt.call_restore(&snap.state);
        self.aliases.insert(snap.uuid.clone(), snap.uuid.clone());
        self.agents.insert(
            snap.uuid.clone(),
            Mounted {
                uuid: snap.uuid.clone(),
                alias: snap.uuid.clone(),
                runtime: Box::new(rt),
                service: None,
                code: Some(snap.code.clone()),
                epoch: snap.epoch,
                grant: Grant::full(), // TODO(M2): carry the manifest in the snapshot + re-fit
            },
        );
        self.seen.insert(snap.uuid.clone(), snap.epoch);
        crate::flow!("[{}] ⇇ migrated '{}' arrived (epoch {})", self.label, snap.uuid, snap.epoch);

        // Re-bind at AMS, carrying the handoff so the AMS node moves the TOFU key.
        if self.routes.contains_key("ams") {
            let ho_json = serde_json::to_value(&ho).unwrap_or_default();
            let body = serde_json::json!({
                "agent": snap.uuid, "address": self.addr, "epoch": snap.epoch, "handoff": ho_json
            })
            .to_string();
            self.send_as(&snap.uuid, "ams", b"obj(bind, agent)", body.as_bytes());
        }
    }

    /// Stamp `sender_pub`/`nonce` and sign a message with this node's key (R1).
    fn seal(&self, m: &mut NodeMsg) {
        m.sender_pub = self.key.public_key().to_vec();
        m.nonce = self.key.nonce().to_vec();
        m.sig = Vec::new();
        m.sig = self.key.sign(&signing_bytes(m)).to_vec();
    }

    /// Inject a *local* kickoff to the primary agent (trusted, in-process) —
    /// bypasses the wire gate and runs the executor to completion.
    pub fn inject(&mut self, unl: &[u8], body: &[u8]) {
        let to = self.primary.clone();
        self.pump(NodeMsg { to: to.clone(), from: to, unl: unl.to_vec(), body: body.to_vec(), ..Default::default() });
    }

    /// Serve until `shutdown`: accept connections, gate+deliver messages, answer
    /// RESOLVE requests (from the local AMS agent).
    pub fn serve(&mut self, listener: TcpListener, shutdown: Arc<AtomicBool>) {
        use std::sync::atomic::AtomicUsize;
        const MAX_CONNS: usize = 64; // bound concurrent handshakes under a flood

        listener.set_nonblocking(true).ok();
        let (in_tx, in_rx) = std::sync::mpsc::channel::<NodeMsg>();
        let (rz_tx, rz_rx) = std::sync::mpsc::channel::<(String, Sender<String>)>();
        let (mg_tx, mg_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (llm_tx, llm_rx) = std::sync::mpsc::channel::<(String, u64, String)>();
        let conns = Arc::new(AtomicUsize::new(0));

        while !shutdown.load(Ordering::Relaxed) {
            // 1. Local kickoff injections (trusted, in-process — never the wire).
            let mut kicks = Vec::new();
            if let Some(rx) = &self.kick_rx {
                while let Ok(k) = rx.try_recv() {
                    kicks.push(k);
                }
            }
            for (unl, body) in kicks {
                let to = self.primary.clone();
                self.pump(NodeMsg { to: to.clone(), from: to, unl, body, ..Default::default() });
            }

            // 2. Messages decoded by connection threads → wire gate + executor.
            while let Ok(m) = in_rx.try_recv() {
                self.accept_wire(m);
            }

            // 3. RESOLVE requests from connection threads → answer from local AMS.
            while let Ok((uuid, resp)) = rz_rx.try_recv() {
                let addr = self.resolve_local(&uuid).unwrap_or_default();
                let _ = resp.send(addr);
            }

            // 3b. Migrated agents handed over by a connection thread (M5).
            while let Ok(payload) = mg_rx.try_recv() {
                self.process_migrate(&payload);
            }

            // 3c. Fire any due timers (M3 scheduling — agent autonomy).
            let now = now_ms();
            let mut due: Vec<(String, u64)> = Vec::new();
            for (uuid, slots) in &self.timers {
                for (id, deadline) in slots {
                    if *deadline <= now {
                        due.push((uuid.clone(), *id));
                    }
                }
            }
            for (uuid, id) in due {
                if let Some(slots) = self.timers.get_mut(&uuid) {
                    slots.remove(&id);
                }
                self.fire_tick(&uuid, id);
            }

            // 3d. Run pending inferences off the main thread (M5 llm — async).
            if let Some(backend) = self.llm.clone() {
                for (uuid, req_id, prompt) in std::mem::take(&mut self.pending_infers) {
                    let (b, tx) = (backend.clone(), llm_tx.clone());
                    std::thread::spawn(move || {
                        let text = b.infer(&prompt).unwrap_or_else(|e| format!("error: {e}"));
                        let _ = tx.send((uuid, req_id, text));
                    });
                }
            } else {
                self.pending_infers.clear(); // no backend → drop (no reply)
            }

            // 3e. Deliver completed inferences back as messages from "llm" (the
            //     async reply-by-message model — the agent correlates by request_id).
            while let Ok((uuid, req_id, text)) = llm_rx.try_recv() {
                let body = serde_json::json!({ "request_id": req_id, "text": text }).to_string();
                self.pump(NodeMsg {
                    to: uuid,
                    from: "llm".into(),
                    unl: b"obj(inferred, x)".to_vec(),
                    body: body.into_bytes(),
                    ..Default::default()
                });
            }

            // 4. Accept new connections; each is handshaked + read in its own thread
            //    so a slow peer cannot stall the loop (H3/R7). Shed load past the cap.
            match listener.accept() {
                Ok((s, _)) => {
                    if conns.load(Ordering::Relaxed) >= MAX_CONNS {
                        continue; // drop (the connection closes) — bounded resource use
                    }
                    conns.fetch_add(1, Ordering::Relaxed);
                    let (noise, in_tx, rz_tx, mg_tx, conns2) = (
                        self.noise.clone(),
                        in_tx.clone(),
                        rz_tx.clone(),
                        mg_tx.clone(),
                        conns.clone(),
                    );
                    std::thread::spawn(move || {
                        handle_conn(s, &noise, &in_tx, &rz_tx, &mg_tx);
                        conns2.fetch_sub(1, Ordering::Relaxed);
                    });
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
                self.label,
                msg.from
            );
            return;
        }
        if !self.authorize(&msg) {
            crate::flow!(
                "[{}] ⛔ impersonation of '{}' — sender key ≠ first-seen (TOFU)",
                self.label,
                msg.from
            );
            return;
        }
        self.pump(msg);
    }

    /// R3: trust-on-first-use from-authorization. The first node key seen signing
    /// for a given `from` uuid owns it; a later message claiming that uuid under a
    /// different key is rejected as impersonation (`THREAT_MODEL.md` C1/C2/C5).
    /// Authoritative AMS-distributed keys + owner delegation (`MOBILITY.md` §7)
    /// strengthen this in M5.
    fn authorize(&mut self, m: &NodeMsg) -> bool {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&m.sender_pub); // length already checked in wire_admit
        match self.keys.get(&m.from).copied() {
            None => {
                self.keys.insert(m.from.clone(), pk);
                true
            }
            Some(known) if known == pk => true,
            // Key changed: accept only with a valid handoff from the CURRENT key to
            // this new key (a legitimate migration — MOBILITY §7); else impersonation.
            Some(known) => {
                if self.handoff_authorizes(m, &known, &pk) {
                    self.keys.insert(m.from.clone(), pk);
                    crate::flow!("[{}] ↪ key handoff accepted for '{}'", self.label, m.from);
                    true
                } else {
                    self.audit(&m.from, "impersonation", "sender key != TOFU");
                    false
                }
            }
        }
    }

    /// True if `m.body` carries a handoff signed by `from_key` (the agent's current
    /// authorized key) that authorizes `to_key` (the new sender key) for this agent.
    fn handoff_authorizes(&self, m: &NodeMsg, from_key: &[u8; 32], to_key: &[u8; 32]) -> bool {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&m.body) else { return false };
        let Some(ho_val) = v.get("handoff") else { return false };
        let Ok(ho) = serde_json::from_value::<Handoff>(ho_val.clone()) else { return false };
        ho.verify()
            && ho.agent == m.from
            && ho.from_pub.as_slice() == &from_key[..]
            && ho.to_pub.as_slice() == &to_key[..]
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

    /// True if `to` names a locally-mounted agent (by uuid or alias) → its uuid.
    fn local_uuid(&self, to: &str) -> Option<String> {
        if self.agents.contains_key(to) {
            Some(to.to_string())
        } else {
            self.aliases.get(to).cloned()
        }
    }

    /// The in-process executor. Deliver a message to its local agent; queue any
    /// reply bound for a co-located agent (in-process, no wire) and push any
    /// cross-node reply to the wire. A per-event work budget bounds intra-node
    /// fan-out so a local message loop cannot exhaust the node.
    fn pump(&mut self, initial: NodeMsg) {
        let mut q = std::collections::VecDeque::new();
        q.push_back(initial);
        let mut budget = 10_000usize;
        while let Some(m) = q.pop_front() {
            budget -= 1;
            if budget == 0 {
                crate::flow!("[{}] ⛔ executor budget exhausted — dropping the rest", self.label);
                break;
            }
            let Some(uuid) = self.local_uuid(&m.to) else {
                // Not for any local agent: surface to the sink (e.g. "result") or drop.
                if let Some(sink) = &self.sink {
                    let _ = sink.send(m);
                }
                continue;
            };
            // A quarantined agent (M6) receives nothing further.
            if self.quarantined.contains(&uuid) {
                self.audit(&uuid, "quarantined", "message dropped");
                continue;
            }
            // Cache the sender's return address so replies have a route.
            if !m.from.is_empty() && !m.from_addr.is_empty() {
                self.routes.insert(m.from.clone(), m.from_addr.clone());
            }
            let (result, sends, ops, infers, spawns) = {
                let mounted = self.agents.get_mut(&uuid).expect("local uuid is mounted");
                crate::flow!("[{}] ← {} : {}", mounted.alias, m.from, String::from_utf8_lossy(&m.unl));
                let result = mounted.runtime.config(&m.from, &m.unl, &m.body);
                (
                    result,
                    mounted.runtime.take_sends(),
                    mounted.runtime.take_timer_ops(),
                    mounted.runtime.take_infer_reqs(),
                    mounted.runtime.take_spawn_reqs(),
                )
            };
            self.supervise(&uuid, &result);
            self.apply_timer_ops(&uuid, ops);
            self.apply_infer_reqs(&uuid, infers);
            self.apply_spawn_reqs(&uuid, spawns);
            for s in sends {
                if !self.net_allows(&uuid, &s.receiver) {
                    self.audit(&uuid, "denied:net", &s.receiver);
                    crate::flow!("[{}] ⛔ net-scope denied: '{}' → '{}'", self.label, uuid, s.receiver);
                    continue;
                }
                let next = NodeMsg {
                    to: s.receiver,
                    from: uuid.clone(),
                    from_addr: self.addr.clone(),
                    unl: s.unl,
                    body: s.body,
                    ..Default::default()
                };
                if self.local_uuid(&next.to).is_some() {
                    q.push_back(next); // co-located → in-process, trusted
                } else {
                    self.wire_or_sink(next); // cross-node → seal + Noise, or sink
                }
            }
        }
    }

    /// Seal a cross-node message and send it over Noise; if the recipient has no
    /// address (e.g. `result`), surface it to the sink instead.
    fn wire_or_sink(&mut self, mut m: NodeMsg) {
        self.seal(&mut m);
        match self.address_of(&m.to) {
            Some(addr) => {
                let _ = self.send_to(&addr, &m);
            }
            None => {
                if let Some(sink) = &self.sink {
                    let _ = sink.send(m);
                }
            }
        }
    }

    /// Emit a node-originated message as agent `from` (used by `register`): in-process
    /// if the target is co-located, else over the wire.
    fn send_as(&mut self, from: &str, to: &str, unl: &[u8], body: &[u8]) {
        let m = NodeMsg {
            to: to.into(),
            from: from.into(),
            from_addr: self.addr.clone(),
            unl: unl.to_vec(),
            body: body.to_vec(),
            ..Default::default()
        };
        if self.local_uuid(to).is_some() {
            self.pump(m);
        } else {
            self.wire_or_sink(m);
        }
    }

    /// Apply an agent's timer requests (M3), gated by the `Time` capability and the
    /// per-agent slot budget. Denials are silent to the agent, logged node-side.
    fn apply_timer_ops(&mut self, uuid: &str, ops: Vec<TimerOp>) {
        if ops.is_empty() {
            return;
        }
        if !self.granted(uuid, Capability::Time) {
            crate::flow!("[{}] ⛔ timer denied for '{}' (no Time grant)", self.label, uuid);
            return;
        }
        let budget = self.agents.get(uuid).map(|m| m.grant.budget.timers as usize).unwrap_or(0);
        let now = now_ms();
        let slots = self.timers.entry(uuid.to_string()).or_default();
        for op in ops {
            match op {
                TimerOp::Set { id, delay_ms } => {
                    if !slots.contains_key(&id) && slots.len() >= budget {
                        crate::flow!("[{}] ⛔ timer budget exhausted for '{}'", self.label, uuid);
                        continue;
                    }
                    slots.insert(id, now.saturating_add(delay_ms));
                }
                TimerOp::Cancel { id } => {
                    slots.remove(&id);
                }
            }
        }
    }

    /// Fire a due timer: run the agent's `tick`, then route its sends and apply any
    /// timers it (re-)armed.
    fn fire_tick(&mut self, uuid: &str, timer_id: u64) {
        let now = now_ms();
        let (result, sends, ops, infers, spawns) = {
            let Some(m) = self.agents.get_mut(uuid) else { return };
            let result = m.runtime.tick(timer_id, now);
            (
                result,
                m.runtime.take_sends(),
                m.runtime.take_timer_ops(),
                m.runtime.take_infer_reqs(),
                m.runtime.take_spawn_reqs(),
            )
        };
        self.supervise(uuid, &result);
        self.apply_timer_ops(uuid, ops);
        self.apply_infer_reqs(uuid, infers);
        self.apply_spawn_reqs(uuid, spawns);
        self.dispatch(uuid, sends);
    }

    /// Route a batch of an agent's emitted sends (as `from`): co-located → in-process
    /// via the executor, cross-node → sealed over Noise / sink.
    fn dispatch(&mut self, from: &str, sends: Vec<OutboundIntent>) {
        for s in sends {
            if !self.net_allows(from, &s.receiver) {
                continue;
            }
            let next = NodeMsg {
                to: s.receiver,
                from: from.into(),
                from_addr: self.addr.clone(),
                unl: s.unl,
                body: s.body,
                ..Default::default()
            };
            if self.local_uuid(&next.to).is_some() {
                self.pump(next);
            } else {
                self.wire_or_sink(next);
            }
        }
    }

    /// Send one sealed message to `addr`, reusing a **persistent** Noise channel to
    /// that peer when we have one (the handshake then amortizes over many messages);
    /// a broken channel is transparently re-dialled. The R1 signed envelope travels
    /// inside the R2 encrypted, mutually-authenticated link.
    fn send_to(&mut self, addr: &str, m: &NodeMsg) -> io::Result<()> {
        let frame = encode_msg(m);
        if let Some((s, sess)) = self.conns.get_mut(addr) {
            if sess.send(s, KIND_MSG, &frame).is_ok() {
                return Ok(());
            }
            self.conns.remove(addr); // broken pipe → reconnect below
        }
        let mut s = dial(addr)?;
        let mut sess = self.noise.connect(&mut s)?;
        sess.send(&mut s, KIND_MSG, &frame)?;
        self.conns.insert(addr.to_string(), (s, sess));
        Ok(())
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

    /// Answer a RESOLVE by asking the locally-mounted AMS agent to `locate` the
    /// UUID. Nodes that don't host an `ams` agent produce nothing.
    fn resolve_local(&mut self, uuid: &str) -> Option<String> {
        let ams_uuid = self.aliases.get("ams").cloned()?;
        let mounted = self.agents.get_mut(&ams_uuid)?;
        let body = serde_json::json!({ "agent": uuid }).to_string();
        mounted.runtime.config("resolver", b"obj(locate, agent)", body.as_bytes()).ok()?;
        let reply = mounted.runtime.take_sends().into_iter().next()?;
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
    fn two_local_agents_talk_in_process() {
        // One node hosts both agents; the primary, on a local kick, messages the
        // co-located agent by alias — entirely in-process (no wire, no AMS).
        struct Aye;
        impl Agent for Aye {
            fn on_message(&mut self, unl: &str, _b: &[u8], ctx: &mut Ctx) {
                if unl.contains("kick") {
                    ctx.send("bee", "obj(ping, x)", Vec::new());
                }
            }
        }
        struct Bee;
        impl Agent for Bee {
            fn on_message(&mut self, unl: &str, _b: &[u8], ctx: &mut Ctx) {
                if unl.contains("ping") {
                    ctx.send("result", "obj(pong, x)", Vec::new());
                }
            }
        }
        let (tx, rx) = mpsc::channel();
        let mut n = Node::new("AYE", "aye", "127.0.0.1:0", Box::new(NativeRuntime::new(Aye)));
        n.mount("BEE", "bee", Box::new(NativeRuntime::new(Bee)), None);
        n.set_sink(tx);
        n.inject(b"obj(kick, x)", b""); // local kick → primary → bee → result
        let got = rx.recv_timeout(Duration::from_secs(2)).expect("result surfaced in-process");
        assert_eq!(String::from_utf8_lossy(&got.unl), "obj(pong, x)");
    }

    #[test]
    fn state_capability_persists_per_agent() {
        struct Saver;
        impl Agent for Saver {
            fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
                if unl.contains("save") {
                    ctx.state_put("k", body); // durable, namespace-confined
                } else if unl.contains("load") {
                    let v = ctx.state_get("k").unwrap_or_default();
                    ctx.send("result", "obj(loaded, x)", v);
                }
            }
        }
        let dir = std::env::temp_dir().join(format!("m4-state-{}", std::process::id()));
        let store = crate::adapters::SledStore::open(&dir).unwrap();
        let mut n = Node::new("S", "s", "127.0.0.1:0", Box::new(NativeRuntime::new(Saver)));
        n.set_store(store);
        let (tx, rx) = mpsc::channel();
        n.set_sink(tx);
        // save in one call, load in a later call: state outlives the message
        n.pump(NodeMsg { to: "S".into(), from: "S".into(), unl: b"save".to_vec(), body: b"hello".to_vec(), ..Default::default() });
        n.pump(NodeMsg { to: "S".into(), from: "S".into(), unl: b"load".to_vec(), ..Default::default() });
        let got = rx.recv_timeout(Duration::from_secs(1)).expect("state load surfaced");
        assert_eq!(got.body, b"hello"); // persisted via the state capability
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn audit_records_a_supervised_fault_and_quarantine() {
        struct Rec(std::sync::Mutex<Vec<String>>);
        impl AuditSink for Rec {
            fn record(&self, e: &AuditEvent) {
                self.0.lock().unwrap().push(e.kind.clone());
            }
        }
        struct Boom;
        impl Agent for Boom {
            fn on_message(&mut self, _u: &str, _b: &[u8], _c: &mut Ctx) {
                panic!("boom");
            }
        }
        let rec = Arc::new(Rec(std::sync::Mutex::new(Vec::new())));
        let mut n = Node::new("B", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Boom)));
        n.set_audit(rec.clone());
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // silence the panics
        for _ in 0..3 {
            n.pump(NodeMsg { to: "B".into(), from: "B".into(), unl: b"x".to_vec(), ..Default::default() });
        }
        std::panic::set_hook(prev);
        assert!(n.quarantined.contains("B")); // supervisor isolated the faulting agent
        let kinds = rec.0.lock().unwrap();
        assert!(kinds.iter().any(|k| k == "fault")); // audit recorded the faults
        assert!(kinds.iter().any(|k| k == "quarantined")); // and the quarantine
    }

    #[test]
    fn spawn_restricts_child_caps_to_parent() {
        use crate::manifest::Capability;
        let mut n = Node::new("P", "p", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        // restrict the parent to Spawn + State (NOT Llm)
        n.agents.get_mut("P").unwrap().grant.caps =
            [Capability::Messaging, Capability::Log, Capability::Spawn, Capability::State].into_iter().collect();
        // the child requests State + Llm; only State is within the parent's authority
        let child = wmanifest(&[Capability::State, Capability::Llm]);
        let req = unl_agent::SpawnReq {
            uuid: "CHILD".into(),
            alias: "child".into(),
            code: COUNTER_WASM.as_bytes().to_vec(),
            manifest_json: child.to_json(),
        };
        n.apply_spawn_reqs("P", vec![req]);
        assert!(n.granted("CHILD", Capability::State)); // ⊆ parent → kept
        assert!(!n.granted("CHILD", Capability::Llm)); // not in parent → stripped
    }

    #[test]
    fn crypto_capability_signs_and_verifies() {
        struct Signer;
        impl Agent for Signer {
            fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
                if unl.contains("sign") {
                    match (ctx.sign(body), ctx.crypto_pubkey()) {
                        (Some(sig), Some(pk)) => {
                            let ok = ctx.verify(&pk, body, &sig);
                            ctx.send("result", if ok { "obj(ok, x)" } else { "obj(bad, x)" }, Vec::new());
                        }
                        _ => ctx.send("result", "obj(denied, x)", Vec::new()),
                    }
                }
            }
        }
        let mut n = Node::new("C", "c", "127.0.0.1:0", Box::new(NativeRuntime::new(Signer)));
        let (tx, rx) = mpsc::channel();
        n.set_sink(tx);
        n.pump(NodeMsg { to: "C".into(), from: "C".into(), unl: b"sign".to_vec(), body: b"payload".to_vec(), ..Default::default() });
        let got = rx.recv_timeout(Duration::from_secs(1)).expect("crypto result");
        assert_eq!(String::from_utf8_lossy(&got.unl), "obj(ok, x)"); // node-held key signed + verified
    }

    #[test]
    fn llm_capability_infers_async() {
        struct Echo;
        impl LlmBackend for Echo {
            fn infer(&self, prompt: &str) -> Result<String, String> {
                Ok(format!("REPLY:{prompt}"))
            }
        }
        struct Asker;
        impl Agent for Asker {
            fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut Ctx) {
                if unl.contains("ask") {
                    ctx.infer(42, "hello"); // async — the reply arrives as a message from "llm"
                } else if unl.contains("inferred") {
                    ctx.send("result", "obj(got, x)", body.to_vec()); // surface the reply
                }
            }
        }
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let mut n = Node::new("A", "a", &addr, Box::new(NativeRuntime::new(Asker)));
        n.set_sink(tx);
        n.set_llm(Arc::new(Echo));
        let (ktx, krx) = mpsc::channel();
        n.set_kick(krx);
        let sd = shutdown.clone();
        let h = thread::spawn(move || n.serve(l, sd));

        ktx.send((b"obj(ask, x)".to_vec(), Vec::new())).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(3)).expect("the async llm reply should surface");
        assert!(String::from_utf8_lossy(&got.body).contains("REPLY:hello")); // request_id-correlated reply
        shutdown.store(true, Ordering::Relaxed);
        h.join().ok();
    }

    #[test]
    fn net_scope_none_sandboxes_to_local() {
        let mut n = Node::new("seed", "s", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let mut m = wmanifest(&[]);
        m.budget.net = "none".into();
        n.mount_wasm("W", "w", COUNTER_WASM.as_bytes().to_vec(), &m, None).unwrap();
        assert!(!n.net_allows("W", "ams")); // net=none: cannot reach a remote/platform target
        assert!(n.net_allows("W", "seed")); // a co-located agent → allowed
        assert!(n.net_allows("seed", "ams")); // native full grant (net=platform) → allowed
    }

    #[test]
    fn state_quota_caps_namespace_size() {
        use unl_agent::Kv;
        let dir = std::env::temp_dir().join(format!("m4-quota-{}", std::process::id()));
        let store = std::sync::Arc::new(crate::adapters::SledStore::open(&dir).unwrap());
        let kv = ScopedKv {
            store,
            ns: "A".into(),
            used: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            quota: 8,
        };
        kv.put("k", b"12345678"); // exactly 8 bytes → fits
        assert_eq!(kv.get("k"), Some(b"12345678".to_vec()));
        kv.put("k2", b"x"); // would exceed the 8-byte quota → rejected
        assert_eq!(kv.get("k2"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn timer_fires_a_tick_for_autonomy() {
        struct Ticker;
        impl Agent for Ticker {
            fn on_message(&mut self, unl: &str, _b: &[u8], ctx: &mut Ctx) {
                if unl.contains("arm") {
                    ctx.set_timer(7, 30); // fire in ~30ms, with no further message
                }
            }
            fn on_tick(&mut self, timer_id: u64, _now: u64, ctx: &mut Ctx) {
                ctx.send("result", format!("obj(fired, {timer_id})"), Vec::new());
            }
        }
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let mut n = Node::new("T", "t", &addr, Box::new(NativeRuntime::new(Ticker)));
        n.set_sink(tx);
        let (ktx, krx) = mpsc::channel();
        n.set_kick(krx);
        let sd = shutdown.clone();
        let h = thread::spawn(move || n.serve(l, sd));

        ktx.send((b"obj(arm, x)".to_vec(), Vec::new())).unwrap(); // arm the timer
        let got = rx.recv_timeout(Duration::from_secs(2)).expect("the timer should fire a tick");
        assert_eq!(String::from_utf8_lossy(&got.unl), "obj(fired, 7)");

        shutdown.store(true, Ordering::Relaxed);
        h.join().ok();
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

    // A mobile counter: each deliver increments n; snapshot/restore carry n.
    const COUNTER_WASM: &str = r#"
    (module
      (memory (export "memory") 1)
      (global $n (mut i32) (i32.const 0))
      (global $bump (mut i32) (i32.const 1024))
      (func (export "init"))
      (func (export "alloc") (param $len i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $bump))
        (global.set $bump (i32.add (global.get $bump) (local.get $len)))
        (local.get $p))
      (func (export "deliver") (param i32 i32 i32 i32 i32 i32)
        (global.set $n (i32.add (global.get $n) (i32.const 1))))
      (func (export "snapshot") (result i64)
        (i32.store (i32.const 0) (global.get $n))
        (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const 4)))
      (func (export "restore") (param $p i32) (param $l i32)
        (global.set $n (i32.load (local.get $p)))))
    "#;

    fn wmanifest(grants: &[crate::manifest::Capability]) -> crate::manifest::Manifest {
        use crate::manifest::*;
        Manifest {
            type_id: uuid::Uuid::nil(),
            desc: "counter".into(),
            name: None,
            profile: Profile::Either,
            brain: Brain::Wasm,
            grants: grants.to_vec(),
            budget: Budget::default(),
        }
    }

    #[test]
    fn capability_gate_reflects_the_manifest() {
        use crate::manifest::Capability;
        let mut a = Node::new("seed", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("W", "w", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[Capability::State, Capability::Time]), None)
            .unwrap();
        assert!(a.granted("W", Capability::State)); // requested + offered
        assert!(a.granted("W", Capability::Messaging)); // core
        assert!(!a.granted("W", Capability::Llm)); // not granted → denied
        assert!(a.granted("seed", Capability::Llm)); // native infra = full grant
    }

    #[test]
    fn iot_node_rejects_a_heavy_agent_at_load() {
        use crate::manifest::{Capability, NodeProfile};
        let mut a = Node::new("seed", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.set_profile(NodeProfile::iot());
        // iot offers no llm → load-time rejection, operator-facing
        let r = a.mount_wasm("W", "w", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[Capability::Llm]), None);
        assert!(r.is_err());
    }

    #[test]
    fn wasm_agent_migrates_with_state_between_nodes() {
        // source A hosts a mobile wasm counter, incremented to 3
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        for _ in 0..3 {
            a.pump(NodeMsg { to: "CTR".into(), from: "CTR".into(), unl: b"inc".to_vec(), ..Default::default() });
        }
        // destination B authorizes the move; A builds the signed snapshot + handoff
        let mut b = Node::new("seed-b", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let payload = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();

        b.process_migrate(&payload);
        // B now hosts CTR with the migrated state (n = 3)
        assert_eq!(b.agents.get_mut("CTR").unwrap().runtime.snapshot(), vec![3, 0, 0, 0]);
        assert_eq!(b.seen.get("CTR"), Some(&1));

        // a replay of the same epoch is rejected (E)
        b.process_migrate(&payload);
        assert_eq!(b.seen.get("CTR"), Some(&1));
    }

    #[test]
    fn handoff_authorizes_a_key_change() {
        let mut n = dummy_node();
        let (ka, kb) = (NodeCrypto::generate(), NodeCrypto::generate());
        let sign = |k: &NodeCrypto, from: &str, body: Vec<u8>| {
            let mut m = NodeMsg { to: "ams".into(), from: from.into(), body, ..Default::default() };
            m.sender_pub = k.public_key().to_vec();
            m.nonce = k.nonce().to_vec();
            m.sig = k.sign(&signing_bytes(&m)).to_vec();
            m
        };
        // "X" first seen under key A → A owns it (TOFU)
        let m1 = sign(&ka, "X", Vec::new());
        assert!(n.wire_admit(&m1) && n.authorize(&m1));
        // X under key B with NO handoff → impersonation, rejected
        let m2 = sign(&kb, "X", Vec::new());
        assert!(n.wire_admit(&m2) && !n.authorize(&m2));
        // X under key B WITH a valid handoff A→B → accepted, TOFU moves to B
        let ho = Handoff::sealed("X", kb.public_key().to_vec(), 1, &ka);
        let body = serde_json::json!({ "handoff": serde_json::to_value(&ho).unwrap() }).to_string();
        let m3 = sign(&kb, "X", body.into_bytes());
        assert!(n.wire_admit(&m3) && n.authorize(&m3));
        // B is now the owner: a further message under B (no handoff) is fine
        assert!(n.authorize(&sign(&kb, "X", Vec::new())));
    }
}
