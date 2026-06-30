//! The single-agent event loop. This is the whole "node" for a leaf device: accept
//! a connection, run the Noise handshake, admit one signed message, deliver it to
//! the agent, and route the agent's replies. One agent, one socket at a time —
//! sized for a microcontroller, not a server.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::agent::{Agent, Ctx, Outgoing};
use node_core::crypto::{verify, NodeCrypto};
use node_core::noise::NodeNoise;
use node_core::wire::{decode_msg, encode_msg, signing_bytes, NodeMsg};

/// Frame kind: an application message (the only kind a leaf device needs).
const KIND_MSG: u8 = 1;

/// The node shim: identity, return address, learned routes, and the TOFU key table.
pub struct Shim {
    uuid: String,
    addr: String,
    key: NodeCrypto,
    noise: NodeNoise,
    routes: HashMap<String, String>,
    keys: HashMap<String, [u8; 32]>, // R3: from-uuid -> authorized node key (TOFU)
}

impl Shim {
    /// A shim with fresh ephemeral identities (use [`Shim::with_identity`] to keep a
    /// stable identity across reboots by loading the seed from NVS flash).
    pub fn new(uuid: &str, addr: &str) -> Self {
        Shim::with_identity(uuid, addr, NodeCrypto::generate(), NodeNoise::generate())
    }

    pub fn with_identity(uuid: &str, addr: &str, key: NodeCrypto, noise: NodeNoise) -> Self {
        Shim {
            uuid: uuid.into(),
            addr: addr.into(),
            key,
            noise,
            routes: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    /// Bootstrap a well-known peer (alias/uuid -> address).
    pub fn add_route(&mut self, who: &str, addr: &str) {
        self.routes.insert(who.into(), addr.into());
    }

    /// This node's Ed25519 public key (its signing identity).
    pub fn node_pub(&self) -> [u8; 32] {
        self.key.public_key()
    }

    /// Serve forever. Accepts one connection at a time and handles one signed
    /// message per connection — a leaf device has no fan-out to manage.
    pub fn serve<A: Agent>(&mut self, listener: TcpListener, agent: &mut A) {
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            s.set_read_timeout(Some(Duration::from_secs(30))).ok();
            let Ok(mut sess) = self.noise.accept(&mut s) else { continue };
            let Ok((kind, payload)) = sess.recv(&mut s) else { continue };
            if kind != KIND_MSG {
                continue;
            }
            let Some(m) = decode_msg(&payload) else { continue };
            if !self.admit(&m) {
                continue;
            }
            // Cache the sender's return address so replies have a route.
            if !m.from.is_empty() && !m.from_addr.is_empty() {
                self.routes.insert(m.from.clone(), m.from_addr.clone());
            }
            let unl = String::from_utf8_lossy(&m.unl).into_owned();
            let mut ctx = Ctx::new(&m.from);
            agent.on_message(&unl, &m.body, &mut ctx);
            for o in ctx.take() {
                self.dispatch(o);
            }
        }
    }

    /// Admit one wire message: structurally valid signature + TOFU sender key.
    fn admit(&mut self, m: &NodeMsg) -> bool {
        if m.sig.len() != 64 || m.sender_pub.len() != 32 {
            return false;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&m.sender_pub);
        let mut sg = [0u8; 64];
        sg.copy_from_slice(&m.sig);
        if !verify(&pk, &signing_bytes(m), &sg) {
            return false;
        }
        match self.keys.get(&m.from) {
            None => {
                self.keys.insert(m.from.clone(), pk);
                true
            }
            Some(known) => *known == pk, // key change → impersonation, drop
        }
    }

    /// Seal and send one of the agent's replies to its route (dropped if unknown).
    fn dispatch(&mut self, o: Outgoing) {
        let Some(addr) = self.routes.get(&o.to).cloned() else { return };
        let mut m = NodeMsg {
            to: o.to,
            from: self.uuid.clone(),
            from_addr: self.addr.clone(),
            unl: o.unl.into_bytes(),
            body: o.body,
            ..Default::default()
        };
        self.seal(&mut m);
        let _ = self.send_to(&addr, &m);
    }

    fn seal(&self, m: &mut NodeMsg) {
        m.sender_pub = self.key.public_key().to_vec();
        m.nonce = self.key.nonce().to_vec();
        m.sig = Vec::new();
        m.sig = self.key.sign(&signing_bytes(m)).to_vec();
    }

    fn send_to(&self, addr: &str, m: &NodeMsg) -> std::io::Result<()> {
        let sa = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "unresolvable address"))?;
        let mut s = TcpStream::connect_timeout(&sa, Duration::from_secs(5))?;
        let mut sess = self.noise.connect(&mut s)?;
        sess.send(&mut s, KIND_MSG, &encode_msg(m))
    }
}
