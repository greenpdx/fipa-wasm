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
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::adapters::{self, Engine, HostHooks, Limits, NodeCrypto, NodeNoise, NoiseSession, SledStore, StateStore};
use crate::manifest::{Capability, Grant, Manifest, NodeProfile, Profile};
use crate::wasm::{AgentRuntime, OutboundIntent, WasmRuntime, WasmiEngine};
use rand::RngCore;
use std::collections::HashSet;
use unl_agent::{InferReq, SpawnReq, TimerOp};

use super::migrate::{code_hash, AgentSnapshot, Handoff, MigratePayload};

const KIND_MSG: u8 = 1;
const KIND_RESOLVE_REQ: u8 = 2;
const KIND_RESOLVE_RESP: u8 = 3;
const KIND_MIGRATE: u8 = 4;
const KIND_CODE_FETCH: u8 = 5; // request a wasm module by content hash
const KIND_CODE_BLOB: u8 = 6; // the module bytes (empty = unknown hash)
const KIND_MIGRATE_ACK: u8 = 7; // destination confirms it mounted the migrated agent (prepared)
const KIND_MIGRATE_COMMIT: u8 = 8; // source activates the prepared agent once it has tombstoned

/// A short dial timeout bounds connect/read/write so a slow or hostile peer cannot
/// stall a handler (R4; partial mitigation of `THREAT_MODEL.md` H3). The frame-size
/// cap now lives in the Noise transport ([`crate::adapters::noise`]).
const DIAL_TIMEOUT: Duration = Duration::from_secs(2);

// The signed message envelope and its codec now live in the shared `node-core`
// crate (the same wire the embedded node-shim speaks). Re-export `NodeMsg` so the
// public path `process::node::NodeMsg` is unchanged.
pub use node_core::wire::NodeMsg;
use node_core::wire::{decode_msg, encode_msg, signing_bytes};

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
    in_tx: &SyncSender<NodeMsg>,
    rz_tx: &Sender<(String, Sender<String>)>,
    mg_tx: &Sender<(Vec<u8>, Sender<bool>)>,
    mg_fin_tx: &Sender<(String, bool)>,
    code_store: &Arc<Mutex<HashMap<String, Vec<u8>>>>,
    allow: Option<Arc<HashSet<Vec<u8>>>>,
) {
    s.set_read_timeout(Some(DIAL_TIMEOUT)).ok(); // bound the handshake
    s.set_write_timeout(Some(DIAL_TIMEOUT)).ok();
    let Ok(mut sess) = noise.accept(&mut s) else { return };
    // C2a — enforce the Noise peer allowlist (if configured) before any frame is
    // accepted, so an unknown peer cannot reach the migrate / message handlers.
    if let Some(set) = &allow {
        if !set.contains(sess.peer_static()) {
            return;
        }
    }
    // After the handshake a KIND_MSG channel is persistent, so tolerate idle gaps
    // between messages; a long idle just recycles the connection (sender re-dials).
    s.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let mut frames = 0u32;
    let mut window = std::time::Instant::now();
    loop {
        let (kind, payload) = match sess.recv(&mut s) {
            Ok(v) => v,
            Err(_) => return, // idle timeout / EOF / error → close
        };
        // M1 — per-connection frame-rate cap: an abusive peer that floods frames has
        // its connection dropped rather than driving unbounded work in the main loop.
        if window.elapsed() >= Duration::from_secs(1) {
            window = std::time::Instant::now();
            frames = 0;
        }
        frames += 1;
        if frames > 500 {
            return;
        }
        match kind {
            KIND_MSG => {
                // M1 — cheap structural pre-filter so a flood of malformed frames is
                // dropped in this (parallel) connection thread, never queued; full
                // signature verification + TOFU still run in the main loop.
                if let Some(m) = decode_msg(&payload) {
                    if m.sig.len() == 64 && m.sender_pub.len() == 32 && !adapters::is_reserved_sender(&m.from) {
                        let _ = in_tx.send(m);
                    }
                }
                // keep reading — the channel is persistent
            }
            KIND_MIGRATE => {
                // Decode the uuid up front (payload is moved into the channel) so the
                // finalize step can name the prepared agent.
                let uuid = MigratePayload::decode(&payload).map(|mp| mp.snapshot.uuid).unwrap_or_default();
                let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                if mg_tx.send((payload, resp_tx)).is_ok() {
                    if let Ok(true) = resp_rx.recv_timeout(Duration::from_secs(5)) {
                        // Prepared — confirm so the source tombstones, then await the
                        // COMMIT that activates us. No COMMIT (timeout/EOF) → abort.
                        let _ = sess.send(&mut s, KIND_MIGRATE_ACK, b"");
                        s.set_read_timeout(Some(Duration::from_secs(8))).ok();
                        let commit = matches!(sess.recv(&mut s), Ok((KIND_MIGRATE_COMMIT, _)));
                        let _ = mg_fin_tx.send((uuid, commit));
                    }
                }
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
            KIND_CODE_FETCH => {
                let hash = String::from_utf8_lossy(&payload).to_string();
                let code = code_store.lock().unwrap_or_else(|e| e.into_inner()).get(&hash).cloned().unwrap_or_default();
                let _ = sess.send(&mut s, KIND_CODE_BLOB, &code); // empty = unknown
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
    manifest: Option<Manifest>, // the signed manifest, carried on migration to re-fit (H1)
    active: bool,          // false while a migrated agent is prepared but not yet committed (H3/H4)
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
    code_store: Arc<Mutex<HashMap<String, Vec<u8>>>>,  // content-addressed wasm (CODE_FETCH)
    noise_allow: Option<HashSet<Vec<u8>>>, // C2a: if set, only these peer static keys may connect
    prepared: HashMap<String, Handoff>, // migrated agents mounted-but-suspended, awaiting commit (H3)
    msg_window: HashMap<String, (u64, u32)>, // H5: per-agent egress rate window (start_ms, count)
    nonce_seen: HashSet<(String, Vec<u8>)>, // M5: (from, nonce) replay guard for wire messages
    nonce_order: std::collections::VecDeque<(String, Vec<u8>)>, // M5: eviction order for nonce_seen
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
            code_store: Arc::new(Mutex::new(HashMap::new())),
            noise_allow: None,
            prepared: HashMap::new(),
            msg_window: HashMap::new(),
            nonce_seen: HashSet::new(),
            nonce_order: std::collections::VecDeque::new(),
        };
        node.mount(uuid, alias, agent, None);
        node
    }

    /// Restrict inbound connections to an allowlist of peer Noise static keys (C2a).
    /// Once any peer is allowed, the allowlist is enforced and unknown peers are
    /// dropped right after the handshake. With no allowlist set, any peer may
    /// connect (development default) and authority still rests on the signed
    /// envelope + migration origin checks.
    pub fn allow_noise_peer(&mut self, peer_static: &[u8]) {
        self.noise_allow.get_or_insert_with(HashSet::new).insert(peer_static.to_vec());
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
                manifest: None,       // native agents are stationary (not mobile)
                active: true,
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
        const MAX_INFER_PER_CALL: usize = 16; // M2: bound a single agent's burst
        if reqs.len() > MAX_INFER_PER_CALL {
            self.audit(uuid, "denied:infer-burst", &format!("{} requests", reqs.len()));
        }
        for r in reqs.into_iter().take(MAX_INFER_PER_CALL) {
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

    /// Per-agent egress rate limit (the `msg_per_s` budget, audit H5). A sliding
    /// one-second window keyed on the *sending* agent; over budget → the message is
    /// dropped (and audited), never emitted. Native/infra agents (no manifest) are
    /// trusted and not throttled.
    fn rate_allows(&mut self, uuid: &str) -> bool {
        let limit = match self.agents.get(uuid) {
            Some(m) if m.manifest.is_some() => m.grant.budget.msg_per_s,
            _ => return true, // native/infra or unknown → not rate-limited
        };
        if limit == 0 {
            return true;
        }
        let now = now_ms();
        let slot = self.msg_window.entry(uuid.to_string()).or_insert((now, 0));
        if now.saturating_sub(slot.0) >= 1000 {
            *slot = (now, 0); // new window
        }
        if slot.1 >= limit {
            return false;
        }
        slot.1 += 1;
        true
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
        let runtime = self.instantiate_agent(&code, &grant)?;
        self.aliases.insert(alias.into(), uuid.into());
        self.agents.insert(
            uuid.into(),
            Mounted {
                uuid: uuid.into(),
                alias: alias.into(),
                runtime,
                service: service.map(Into::into),
                code: Some(code),
                epoch: 0,
                grant,
                manifest: Some(manifest.clone()),
                active: true,
            },
        );
        self.provision_state(uuid);
        self.provision_crypto(uuid);
        Ok(())
    }

    /// Instantiate (and `init`) a wasm agent under `grant`, selecting the engine by
    /// node profile: the wasmi interpreter on an IoT node, wasmtime otherwise — the
    /// same agent ABI runs on either (E2). The wasm engine caps are derived from the
    /// granted budget, so both `mount_wasm` and the migration path are sandboxed by
    /// the *fitted* budget rather than defaults (audit H1).
    fn instantiate_agent(&self, code: &[u8], grant: &Grant) -> anyhow::Result<Box<dyn AgentRuntime + Send>> {
        if self.profile.profile == Profile::Iot {
            let limits = Limits {
                fuel: grant.budget.fuel,
                mem_bytes: grant.budget.mem_kb.saturating_mul(1024) as usize,
            };
            let mut m = WasmiEngine.instantiate(code, limits, HostHooks::default())?;
            m.init()?;
            Ok(Box::new(m))
        } else {
            let caps = crate::proto::AgentCapabilities {
                max_memory_bytes: grant.budget.mem_kb.saturating_mul(1024),
                max_execution_time_ms: (grant.budget.fuel / 1_000_000).max(1),
                storage_quota_bytes: grant.budget.state_kb.saturating_mul(1024),
                ..Default::default()
            };
            let mut rt = WasmRuntime::new(code, &caps)?;
            rt.call_init()?;
            Ok(Box::new(rt))
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

    /// Cache a wasm module by its content hash (so this node can serve CODE_FETCH).
    pub fn cache_code(&self, code: Vec<u8>) {
        self.code_store.lock().unwrap_or_else(|e| e.into_inner()).insert(code_hash(&code), code);
    }

    /// Fetch a wasm module by content `hash` from the peer at `addr`, verify it
    /// content-addresses, and cache it. `None` if the peer lacks it.
    pub fn fetch_code(&mut self, addr: &str, hash: &str) -> Option<Vec<u8>> {
        let mut s = dial(addr).ok()?;
        let mut sess = self.noise.connect(&mut s).ok()?;
        sess.send(&mut s, KIND_CODE_FETCH, hash.as_bytes()).ok()?;
        let (kind, code) = sess.recv(&mut s).ok()?;
        if kind != KIND_CODE_BLOB || code.is_empty() || code_hash(&code) != hash {
            return None;
        }
        self.cache_code(code.clone());
        Some(code)
    }

    /// Build the signed move payload (snapshot of code+state at epoch+1, plus a
    /// handoff authorizing `dest_pub`) for a mobile agent — without sending it. The
    /// code is cached so the destination can CODE_FETCH it.
    pub fn build_migrate_payload(&mut self, uuid: &str, dest_pub: &[u8]) -> Option<Vec<u8>> {
        let (code, epoch, state, manifest_json) = {
            let m = self.agents.get_mut(uuid)?;
            let code = m.code.clone()?; // only wasm (mobile) agents have code
            let manifest_json = m.manifest.as_ref()?.to_json(); // mobile agents carry a manifest
            let epoch = m.epoch + 1;
            m.epoch = epoch; // persist the bump so a retried migration advances past `seen` (H3)
            (code, epoch, m.runtime.snapshot(), manifest_json)
        };
        self.cache_code(code.clone());
        let snapshot = AgentSnapshot::sealed(uuid, epoch, code, state, manifest_json, &self.key);
        let handoff = Handoff::sealed(uuid, dest_pub.to_vec(), epoch, &self.key);
        Some(MigratePayload { snapshot, handoff, from_addr: self.addr.clone() }.encode())
    }

    /// Migrate a mobile (wasm) agent to `dest_addr`, authorizing `dest_pub` to act
    /// for it: send the signed snapshot + handoff over Noise, then tombstone the
    /// local copy. The epoch arbiter (R6) + handoff prevent forking (H1); full
    /// two-phase crash-safety is the remaining hardening.
    pub fn migrate(&mut self, uuid: &str, dest_addr: &str, dest_pub: &[u8]) -> io::Result<()> {
        let payload = self
            .build_migrate_payload(uuid, dest_pub)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "agent not mobile / absent"))?;
        // Suspend the local agent for the whole handoff so it never runs concurrently
        // with the destination's (prepared) copy — no double execution / fork (H4).
        if let Some(m) = self.agents.get_mut(uuid) {
            m.active = false;
        }
        match self.send_migration(dest_addr, &payload) {
            Ok(()) => {
                // The destination has prepared the agent and we have told it to
                // commit; tombstone the local copy (no loss — see send_migration).
                if let Some(m) = self.agents.remove(uuid) {
                    self.aliases.remove(&m.alias);
                }
                Ok(())
            }
            Err(e) => {
                // The move did not complete: resume the local agent (no loss, no fork —
                // the destination never activates without our COMMIT).
                if let Some(m) = self.agents.get_mut(uuid) {
                    m.active = true;
                }
                Err(e)
            }
        }
    }

    /// The migration exchange (source side). Send the signed payload, await the
    /// destination's prepare-ACK with a timeout **strictly greater** than the
    /// destination's mount budget so a busy destination cannot time us out into a
    /// fork (H3), then send the COMMIT that activates the prepared agent. Returns
    /// `Ok` only once the destination has acknowledged the prepared mount; the
    /// caller then tombstones. On any error the caller resumes the local agent, and
    /// the destination — having received no COMMIT — aborts its prepared copy.
    fn send_migration(&self, dest_addr: &str, payload: &[u8]) -> io::Result<()> {
        let mut s = dial(dest_addr)?;
        s.set_read_timeout(Some(Duration::from_secs(8))).ok(); // > the destination's 5s wait (H3)
        let mut sess = self.noise.connect(&mut s)?;
        sess.send(&mut s, KIND_MIGRATE, payload)?;
        let (kind, _) = sess.recv(&mut s)?;
        if kind != KIND_MIGRATE_ACK {
            return Err(io::Error::new(io::ErrorKind::Other, "migration not acknowledged"));
        }
        sess.send(&mut s, KIND_MIGRATE_COMMIT, b"")?;
        Ok(())
    }

    /// Receive a migrated agent: verify the snapshot + handoff, confirm it is for
    /// this node, guard against replay (epoch must advance), instantiate the wasm
    /// from the carried code, restore state, mount it, and re-bind at AMS carrying
    /// the handoff so the AMS node can move the agent's authorized key.
    /// Returns `true` only when *this* payload mounts (prepares) an agent, so the
    /// ACK reflects an actual mount rather than the mere presence of some agent with
    /// that uuid (H2). The agent is mounted **suspended** and is not activated (nor
    /// re-bound at AMS, nor recorded in `seen`) until [`Node::commit_migrated`].
    fn process_migrate(&mut self, payload: &[u8]) -> bool {
        let Some(mp) = MigratePayload::decode(payload) else { return false };
        let from_addr = mp.from_addr.clone();
        let (snap, ho) = (mp.snapshot, mp.handoff);
        if !snap.verify() || !ho.verify() {
            crate::flow!("[{}] ⛔ migrate: bad snapshot/handoff signature", self.label);
            return false;
        }
        if ho.to_pub != self.key.public_key()
            || ho.agent != snap.uuid
            || ho.epoch != snap.epoch
            || ho.from_pub != snap.origin_pub
        {
            crate::flow!("[{}] ⛔ migrate: handoff not for me / inconsistent", self.label);
            return false;
        }
        // C2 — reject reserved system ids and refuse to overwrite a locally-born
        // (non-migrated) agent, so an unauthenticated peer cannot hijack an identity.
        if adapters::is_reserved_sender(&snap.uuid) {
            crate::flow!("[{}] ⛔ migrate: reserved id '{}' refused", self.label, snap.uuid);
            return false;
        }
        if self.agents.contains_key(&snap.uuid) && !self.seen.contains_key(&snap.uuid) {
            crate::flow!("[{}] ⛔ migrate: '{}' collides with a local agent", self.label, snap.uuid);
            return false;
        }
        // C2 — origin authenticity: the snapshot must be signed by the agent's
        // currently-authorized node key. A known key MUST match; first sighting is
        // TOFU-accepted only behind the Noise peer allowlist (enforced at accept).
        // TODO(attestation): replace first-sighting TOFU with an authoritative AMS
        // key lookup (MOBILITY §7) — staged follow-up behind this same check.
        if let Some(known) = self.keys.get(&snap.uuid).copied() {
            if known.as_slice() != snap.origin_pub.as_slice() {
                self.audit(&snap.uuid, "migrate:bad-origin", "origin key != authorized key");
                crate::flow!("[{}] ⛔ migrate: origin key ≠ authorized key for '{}'", self.label, snap.uuid);
                return false;
            }
        }
        // Replay guard: reject a non-advancing epoch, consulting both the in-memory
        // and the durable (M4) record so a captured payload cannot re-mount after a
        // restart wiped `seen`.
        let last = self.seen.get(&snap.uuid).copied().or_else(|| self.persisted_seen(&snap.uuid));
        if last.is_some_and(|e| snap.epoch <= e) {
            crate::flow!("[{}] ⛔ migrate: replayed epoch {} for '{}'", self.label, snap.epoch, snap.uuid);
            return false;
        }
        // H1 — re-fit the carried manifest against THIS node's profile; migration
        // never inherits authority and never runs at Grant::full().
        let Some(manifest) = Manifest::from_json(&snap.manifest) else {
            crate::flow!("[{}] ⛔ migrate: missing/invalid manifest", self.label);
            return false;
        };
        let grant = match self.profile.fit(&manifest) {
            Ok(g) => g,
            Err(e) => {
                self.audit(&snap.uuid, "migrate:unfit", &format!("{e:?}"));
                crate::flow!("[{}] ⛔ migrate: manifest does not fit node profile: {e:?}", self.label);
                return false;
            }
        };
        // resolve the wasm: inline if present (cache it), else CODE_FETCH from origin
        let code = if !snap.code.is_empty() {
            self.cache_code(snap.code.clone());
            snap.code.clone()
        } else {
            // SSRF guard (M6): only CODE_FETCH-dial an address we already know as a
            // route, so an attacker-supplied `from_addr` cannot make us connect to an
            // arbitrary (e.g. internal) host. The content hash is verified by fetch_code.
            if from_addr.is_empty() || !self.routes.values().any(|a| a == &from_addr) {
                crate::flow!("[{}] ⛔ migrate: CODE_FETCH to unknown address refused", self.label);
                return false;
            }
            match self.fetch_code(&from_addr, &snap.code_hash) {
                Some(c) => c,
                None => {
                    crate::flow!("[{}] ⛔ migrate: code unavailable (CODE_FETCH failed)", self.label);
                    return false;
                }
            }
        };
        let mut runtime = match self.instantiate_agent(&code, &grant) {
            Ok(rt) => rt,
            Err(_) => {
                crate::flow!("[{}] ⛔ migrate: code won't instantiate", self.label);
                return false;
            }
        };
        runtime.restore(&snap.state);
        // Pin the origin key for this agent (first sighting) so a later migration
        // under a different key is rejected as impersonation.
        let mut origin = [0u8; 32];
        origin.copy_from_slice(&snap.origin_pub);
        self.keys.entry(snap.uuid.clone()).or_insert(origin);
        self.aliases.insert(snap.uuid.clone(), snap.uuid.clone());
        self.agents.insert(
            snap.uuid.clone(),
            Mounted {
                uuid: snap.uuid.clone(),
                alias: snap.uuid.clone(),
                runtime,
                service: None,
                code: Some(code),
                epoch: snap.epoch,
                grant,
                manifest: Some(manifest),
                active: false, // prepared: not live until the source COMMITs (H3/H4)
            },
        );
        self.provision_state(&snap.uuid);
        self.provision_crypto(&snap.uuid);
        // Stash the handoff for the AMS re-bind that happens at commit; `seen` and
        // the AMS binding are deferred so an aborted prepare leaves no trace.
        self.prepared.insert(snap.uuid.clone(), ho);
        crate::flow!("[{}] ⇉ migrated '{}' prepared (epoch {})", self.label, snap.uuid, snap.epoch);
        true
    }

    /// Finalize a prepared migration: activate the agent, record the epoch in the
    /// replay guard, and re-bind it at AMS carrying the handoff (H3). Called when the
    /// source confirms — by COMMIT — that it has tombstoned its copy.
    fn commit_migrated(&mut self, uuid: &str) {
        let epoch = match self.agents.get_mut(uuid) {
            Some(m) if !m.active => {
                m.active = true;
                m.epoch
            }
            _ => return, // unknown or already committed
        };
        self.seen.insert(uuid.to_string(), epoch);
        self.persist_seen(uuid, epoch); // M4: survive a restart
        crate::flow!("[{}] ⇇ migrated '{}' committed (epoch {})", self.label, uuid, epoch);
        if let Some(ho) = self.prepared.remove(uuid) {
            if self.routes.contains_key("ams") {
                let ho_json = serde_json::to_value(&ho).unwrap_or_default();
                let body = serde_json::json!({
                    "agent": uuid, "address": self.addr, "epoch": epoch, "handoff": ho_json
                })
                .to_string();
                self.send_as(uuid, "ams", b"obj(bind, agent)", body.as_bytes());
            }
        }
    }

    /// Discard a prepared migration that the source never committed (the source
    /// timed out, errored, or chose to keep its copy) — leaving no trace, so a clean
    /// retry can prepare again.
    fn abort_prepared(&mut self, uuid: &str) {
        match self.agents.get(uuid) {
            Some(m) if !m.active => {}
            _ => return, // unknown or already committed — never tear down a live agent
        }
        if let Some(m) = self.agents.remove(uuid) {
            self.aliases.remove(&m.alias);
        }
        self.prepared.remove(uuid);
        crate::flow!("[{}] ⛔ migrated '{}' aborted (no commit)", self.label, uuid);
    }

    /// Persist a committed migration epoch so the replay guard survives a restart
    /// (M4): an in-memory `seen` alone lets a captured payload re-mount after a crash.
    fn persist_seen(&self, uuid: &str, epoch: u64) {
        if let Some(store) = &self.store {
            let _ = store.put("_migrate_seen", uuid, &epoch.to_be_bytes());
        }
    }

    /// The durably-recorded last migration epoch for `uuid`, if any (M4).
    fn persisted_seen(&self, uuid: &str) -> Option<u64> {
        let bytes = self.store.as_ref()?.get("_migrate_seen", uuid).ok().flatten()?;
        (bytes.len() == 8).then(|| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes);
            u64::from_be_bytes(b)
        })
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

        const MAX_PER_IP: usize = 8; // bound connections from any single source (M8)
        const MAX_INFLIGHT_INFER: usize = 32; // bound concurrent llm worker threads (M2)

        listener.set_nonblocking(true).ok();
        // Bounded inbound queue (M1): a flooding peer blocks on send (backpressure)
        // instead of growing node memory without limit.
        let (in_tx, in_rx) = std::sync::mpsc::sync_channel::<NodeMsg>(1024);
        let (rz_tx, rz_rx) = std::sync::mpsc::channel::<(String, Sender<String>)>();
        let (mg_tx, mg_rx) = std::sync::mpsc::channel::<(Vec<u8>, Sender<bool>)>();
        let (mg_fin_tx, mg_fin_rx) = std::sync::mpsc::channel::<(String, bool)>(); // (uuid, commit?)
        let (llm_tx, llm_rx) = std::sync::mpsc::channel::<(String, u64, String)>();
        let conns = Arc::new(AtomicUsize::new(0));
        let infl = Arc::new(AtomicUsize::new(0)); // in-flight inferences (M2)
        let per_ip = Arc::new(Mutex::new(HashMap::<std::net::IpAddr, usize>::new())); // M8
        let allow = self.noise_allow.clone().map(Arc::new); // C2a: shared per-connection

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

            // 3b. Migrated agents handed over by a connection thread; ACK only if
            //     THIS payload actually prepared a mount (H2), so a rejected/replayed
            //     payload never elicits a spurious ACK that would tombstone the source.
            while let Ok((payload, resp)) = mg_rx.try_recv() {
                let mounted = self.process_migrate(&payload);
                let _ = resp.send(mounted);
            }

            // 3b'. Finalize a prepared migration once the source decides: commit
            //      (activate + AMS re-bind) on COMMIT, else abort (drop the prepared
            //      copy). The agent is live on exactly one node across the handoff.
            while let Ok((uuid, commit)) = mg_fin_rx.try_recv() {
                if commit {
                    self.commit_migrated(&uuid);
                } else {
                    self.abort_prepared(&uuid);
                }
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

            // 3d. Run pending inferences off the main thread (llm — async). M2: bound
            //     concurrent worker threads; anything over the cap stays queued for a
            //     later iteration rather than spawning unbounded threads.
            if let Some(backend) = self.llm.clone() {
                for (uuid, req_id, prompt) in std::mem::take(&mut self.pending_infers) {
                    if infl.load(Ordering::Relaxed) >= MAX_INFLIGHT_INFER {
                        self.pending_infers.push((uuid, req_id, prompt)); // re-queue; retry next loop
                        continue;
                    }
                    infl.fetch_add(1, Ordering::Relaxed);
                    let (b, tx, infl2) = (backend.clone(), llm_tx.clone(), infl.clone());
                    std::thread::spawn(move || {
                        let text = b.infer(&prompt).unwrap_or_else(|e| format!("error: {e}"));
                        let _ = tx.send((uuid, req_id, text));
                        infl2.fetch_sub(1, Ordering::Relaxed);
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
                    // M8 — per-source-IP cap: a single host cannot pin all the slots.
                    let ip = s.peer_addr().ok().map(|a| a.ip());
                    if let Some(ip) = ip {
                        let mut map = per_ip.lock().unwrap_or_else(|e| e.into_inner());
                        let c = map.entry(ip).or_insert(0);
                        if *c >= MAX_PER_IP {
                            continue; // too many from this IP → drop
                        }
                        *c += 1;
                    }
                    conns.fetch_add(1, Ordering::Relaxed);
                    let (noise, in_tx, rz_tx, mg_tx, mg_fin_tx, cs, conns2, al, pip) = (
                        self.noise.clone(),
                        in_tx.clone(),
                        rz_tx.clone(),
                        mg_tx.clone(),
                        mg_fin_tx.clone(),
                        self.code_store.clone(),
                        conns.clone(),
                        allow.clone(),
                        per_ip.clone(),
                    );
                    std::thread::spawn(move || {
                        handle_conn(s, &noise, &in_tx, &rz_tx, &mg_tx, &mg_fin_tx, &cs, al);
                        conns2.fetch_sub(1, Ordering::Relaxed);
                        if let Some(ip) = ip {
                            let mut map = pip.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(c) = map.get_mut(&ip) {
                                *c -= 1;
                                if *c == 0 {
                                    map.remove(&ip);
                                }
                            }
                        }
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
        // M5 — reject a replayed envelope: a (from, nonce) pair seen before is a
        // re-sent, still-valid message (the signed nonce gives per-message freshness).
        if !msg.nonce.is_empty() {
            let key = (msg.from.clone(), msg.nonce.clone());
            if self.nonce_seen.contains(&key) {
                self.audit(&msg.from, "replay", "duplicate nonce");
                crate::flow!("[{}] ⛔ replayed envelope from '{}'", self.label, msg.from);
                return;
            }
            self.nonce_seen.insert(key.clone());
            self.nonce_order.push_back(key);
            if self.nonce_order.len() > 8192 {
                if let Some(old) = self.nonce_order.pop_front() {
                    self.nonce_seen.remove(&old);
                }
            }
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
        // L4 — tie the handoff to the bind it rides on: its epoch must match the
        // bind's epoch, so a once-valid handoff cannot be replayed onto a different
        // bind. (When the bind carries no epoch, fall back to the handoff's own.)
        let bind_epoch = v.get("epoch").and_then(|e| e.as_u64()).unwrap_or(ho.epoch);
        ho.verify()
            && ho.epoch == bind_epoch
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
            // A migrated agent that is prepared-but-not-committed is not yet live
            // (H3/H4): hold delivery until it commits or is aborted.
            if !self.agents.get(&uuid).map(|m| m.active).unwrap_or(false) {
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
                if !self.rate_allows(&uuid) {
                    self.audit(&uuid, "denied:rate", &s.receiver);
                    crate::flow!("[{}] ⛔ msg-rate denied for '{}'", self.label, uuid);
                    continue;
                }
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
            if !m.active {
                return; // a prepared-but-uncommitted migrated agent does not tick (H4)
            }
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
            if !self.rate_allows(from) {
                self.audit(from, "denied:rate", &s.receiver);
                continue;
            }
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
    fn msg_rate_is_capped_per_window() {
        let mut n = Node::new("seed", "s", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let mut m = wmanifest(&[]);
        m.budget.msg_per_s = 3; // tiny egress budget
        n.mount_wasm("W", "w", COUNTER_WASM.as_bytes().to_vec(), &m, None).unwrap();
        // the first 3 emits in the window pass; the 4th is throttled (H5)
        assert!(n.rate_allows("W"));
        assert!(n.rate_allows("W"));
        assert!(n.rate_allows("W"));
        assert!(!n.rate_allows("W"));
        // a native/infra agent (no manifest) is trusted and never throttled
        for _ in 0..100 {
            assert!(n.rate_allows("seed"));
        }
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
    fn migrate_tombstones_only_after_destination_acks() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let dst = Node::new("d-seed", "d", &addr, Box::new(NativeRuntime::new(Ponger)));
        let dst_pub = dst.node_pub();
        let sd = shutdown.clone();
        let h = thread::spawn(move || {
            let mut dst = dst;
            dst.serve(l, sd);
        });
        thread::sleep(Duration::from_millis(50));

        let mut src = Node::new("s-seed", "s", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        src.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        src.migrate("CTR", &addr, &dst_pub).unwrap(); // round-trips; tombstones on ack
        assert!(!src.agents.contains_key("CTR")); // dropped only after the dest confirmed

        // a migration to an unreachable destination FAILS and keeps the agent (no loss)
        src.mount_wasm("CTR2", "ctr2", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        assert!(src.migrate("CTR2", "127.0.0.1:1", &dst_pub).is_err());
        assert!(src.agents.contains_key("CTR2")); // kept — not lost

        shutdown.store(true, Ordering::Relaxed);
        h.join().ok();
    }

    #[test]
    fn code_fetch_serves_wasm_by_hash() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let shutdown = Arc::new(AtomicBool::new(false));
        let code = COUNTER_WASM.as_bytes().to_vec();
        let hash = code_hash(&code);
        let src = Node::new("S", "s", &addr, Box::new(NativeRuntime::new(Ponger)));
        src.cache_code(code.clone()); // origin caches the module by content hash
        let sd = shutdown.clone();
        let h = thread::spawn(move || {
            let mut src = src;
            src.serve(l, sd);
        });
        thread::sleep(Duration::from_millis(50));

        let mut dst = Node::new("D", "d", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        assert_eq!(dst.fetch_code(&addr, &hash), Some(code.clone())); // fetched by content hash
        assert_eq!(dst.fetch_code(&addr, "deadbeef00"), None); // unknown hash → nothing

        shutdown.store(true, Ordering::Relaxed);
        h.join().ok();
    }

    #[test]
    fn iot_node_mounts_a_wasm_agent_via_the_wasmi_backend() {
        use crate::manifest::{Budget, NodeProfile};
        let mut n = Node::new("seed", "s", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        n.set_profile(NodeProfile::iot());
        let mut m = wmanifest(&[]);
        m.budget = Budget { mem_kb: 256, fuel: 1_000_000, state_kb: 64, timers: 2, msg_per_s: 50, net: "platform".into() };
        let code = wat::parse_str(COUNTER_WASM).unwrap(); // wasmi needs binary wasm
        n.mount_wasm("CTR", "ctr", code, &m, None).unwrap(); // → wasmi interpreter
        n.pump(NodeMsg { to: "CTR".into(), from: "CTR".into(), unl: b"inc".to_vec(), ..Default::default() });
        // the wasmi-backed agent incremented; read its state back
        assert_eq!(n.agents.get_mut("CTR").unwrap().runtime.snapshot(), vec![1, 0, 0, 0]);
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

        assert!(b.process_migrate(&payload)); // prepared (suspended)
        b.commit_migrated("CTR"); // finalize: activate + record the epoch
        // B now hosts CTR with the migrated state (n = 3)
        assert_eq!(b.agents.get_mut("CTR").unwrap().runtime.snapshot(), vec![3, 0, 0, 0]);
        assert_eq!(b.seen.get("CTR"), Some(&1));

        // a replay of the same epoch is rejected (E) — and does not prepare a mount
        assert!(!b.process_migrate(&payload));
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

    #[test]
    fn replayed_envelope_is_dropped() {
        // A signed wire message delivered once must not be re-delivered when the same
        // (from, nonce) envelope is replayed (audit M5).
        let (tx, rx) = mpsc::channel();
        let mut n = Node::new("N", "n", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        n.set_sink(tx);
        let k = NodeCrypto::generate();
        let mut m = NodeMsg { to: "N".into(), from: "X".into(), unl: b"obj(ping, x)".to_vec(), ..Default::default() };
        m.sender_pub = k.public_key().to_vec();
        m.nonce = k.nonce().to_vec();
        m.sig = k.sign(&signing_bytes(&m)).to_vec();

        n.accept_wire(m.clone()); // delivered → Ponger replies pong (no route to X → sink)
        assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok());
        n.accept_wire(m); // exact replay → dropped
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    // ── Phase-1 security regressions (audit C2 / H1) ──────────────────────

    #[test]
    fn migrated_agent_lands_with_a_fitted_grant_not_full() {
        use crate::manifest::Capability;
        // The agent requests only State; on arrival it must hold State + core, and
        // NOT the full grant the old code handed every migrated agent (H1).
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[Capability::State]), None)
            .unwrap();
        let mut b = Node::new("seed-b", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let payload = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();
        b.process_migrate(&payload);
        assert!(b.agents.contains_key("CTR"));
        assert!(b.granted("CTR", Capability::State)); // requested → granted
        assert!(b.granted("CTR", Capability::Messaging)); // core
        assert!(!b.granted("CTR", Capability::Crypto)); // NOT Grant::full() anymore
        assert!(!b.granted("CTR", Capability::Llm));
        assert!(!b.granted("CTR", Capability::Spawn));
    }

    #[test]
    fn migration_refuses_a_manifest_that_does_not_fit_the_destination() {
        use crate::manifest::{Capability, NodeProfile};
        // Source (normal) hosts an Llm-granted agent; the IoT destination offers no
        // Llm, so the re-fit must reject the migration (H1 — no authority inheritance).
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[Capability::Llm]), None)
            .unwrap();
        let mut dst = Node::new("seed-d", "d", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        dst.set_profile(NodeProfile::iot());
        let payload = a.build_migrate_payload("CTR", &dst.node_pub()).unwrap();
        dst.process_migrate(&payload);
        assert!(!dst.agents.contains_key("CTR")); // Llm doesn't fit IoT → refused
    }

    #[test]
    fn migration_refuses_a_reserved_uuid() {
        // A self-signed payload (attacker key) for the reserved id "ams" must be
        // refused even though its signatures verify (C2 — no system-agent hijack).
        let attacker = NodeCrypto::generate();
        let mut dst = Node::new("seed-d", "d", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let manifest = wmanifest(&[]).to_json();
        let snap =
            AgentSnapshot::sealed("ams", 1, COUNTER_WASM.as_bytes().to_vec(), Vec::new(), manifest, &attacker);
        let ho = Handoff::sealed("ams", dst.node_pub().to_vec(), 1, &attacker);
        let payload = MigratePayload { snapshot: snap, handoff: ho, from_addr: dst.addr.clone() }.encode();
        dst.process_migrate(&payload);
        assert!(!dst.agents.contains_key("ams"));
    }

    #[test]
    fn migration_refuses_an_origin_key_change() {
        // First migration pins CTR's origin key; a later self-signed payload under a
        // different key (even at a higher epoch) is rejected as impersonation (C2).
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        let mut b = Node::new("seed-b", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let p1 = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();
        assert!(b.process_migrate(&p1));
        b.commit_migrated("CTR");
        assert!(b.agents.contains_key("CTR")); // first sighting accepted, origin pinned

        let attacker = NodeCrypto::generate();
        let manifest = wmanifest(&[]).to_json();
        let snap =
            AgentSnapshot::sealed("CTR", 99, COUNTER_WASM.as_bytes().to_vec(), Vec::new(), manifest, &attacker);
        let ho = Handoff::sealed("CTR", b.node_pub().to_vec(), 99, &attacker);
        let p2 = MigratePayload { snapshot: snap, handoff: ho, from_addr: b.addr.clone() }.encode();
        assert!(!b.process_migrate(&p2)); // impostor key → not prepared
        assert_eq!(b.seen.get("CTR"), Some(&1)); // epoch-99 impostor rejected; pin held
    }

    // ── Phase-2 two-phase migration (audit H2 / H3 / H4) ──────────────────

    #[test]
    fn a_prepared_migration_does_not_run_until_committed() {
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        let mut b = Node::new("seed-b", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let payload = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();
        assert!(b.process_migrate(&payload)); // prepared, suspended
        assert!(b.agents.contains_key("CTR"));
        assert!(!b.agents.get("CTR").unwrap().active); // not live yet (H4)

        // Delivery to a prepared agent is held — its state must not advance.
        b.pump(NodeMsg { to: "CTR".into(), from: "CTR".into(), unl: b"inc".to_vec(), ..Default::default() });
        assert_eq!(b.agents.get_mut("CTR").unwrap().runtime.snapshot(), vec![0, 0, 0, 0]);

        b.commit_migrated("CTR");
        assert!(b.agents.get("CTR").unwrap().active);
        b.pump(NodeMsg { to: "CTR".into(), from: "CTR".into(), unl: b"inc".to_vec(), ..Default::default() });
        assert_eq!(b.agents.get_mut("CTR").unwrap().runtime.snapshot(), vec![1, 0, 0, 0]); // now it runs
    }

    #[test]
    fn an_aborted_migration_leaves_no_trace_and_allows_retry() {
        let mut a = Node::new("seed-a", "a", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        a.mount_wasm("CTR", "ctr", COUNTER_WASM.as_bytes().to_vec(), &wmanifest(&[]), None).unwrap();
        let mut b = Node::new("seed-b", "b", "127.0.0.1:0", Box::new(NativeRuntime::new(Ponger)));
        let payload = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();
        assert!(b.process_migrate(&payload));
        b.abort_prepared("CTR");
        assert!(!b.agents.contains_key("CTR")); // prepared mount torn down
        assert!(b.seen.get("CTR").is_none()); // no replay-guard trace left behind
        // A clean retry (epoch bumped) prepares again.
        let payload2 = a.build_migrate_payload("CTR", &b.node_pub()).unwrap();
        assert!(b.process_migrate(&payload2));
    }

    #[test]
    fn handoff_with_mismatched_epoch_is_rejected() {
        // A handoff authorizes a key change only for the bind epoch it rides on; a
        // bind body claiming a different epoch must be rejected (audit L4).
        let mut n = dummy_node();
        let (ka, kb) = (NodeCrypto::generate(), NodeCrypto::generate());
        let sign = |k: &NodeCrypto, from: &str, body: Vec<u8>| {
            let mut m = NodeMsg { to: "ams".into(), from: from.into(), body, ..Default::default() };
            m.sender_pub = k.public_key().to_vec();
            m.nonce = k.nonce().to_vec();
            m.sig = k.sign(&signing_bytes(&m)).to_vec();
            m
        };
        let m1 = sign(&ka, "X", Vec::new());
        assert!(n.wire_admit(&m1) && n.authorize(&m1)); // X owned by A (TOFU)

        let ho = Handoff::sealed("X", kb.public_key().to_vec(), 1, &ka); // handoff at epoch 1
        let bad = serde_json::json!({ "epoch": 2, "handoff": serde_json::to_value(&ho).unwrap() }).to_string();
        let m2 = sign(&kb, "X", bad.into_bytes());
        assert!(n.wire_admit(&m2) && !n.authorize(&m2)); // bind epoch 2 ≠ handoff epoch 1 → rejected

        let good = serde_json::json!({ "epoch": 1, "handoff": serde_json::to_value(&ho).unwrap() }).to_string();
        let m3 = sign(&kb, "X", good.into_bytes());
        assert!(n.wire_admit(&m3) && n.authorize(&m3)); // epochs match → accepted
    }
}
