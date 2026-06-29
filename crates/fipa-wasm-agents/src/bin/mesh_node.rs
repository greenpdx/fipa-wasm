// mesh_node.rs — the base FIPA node: hosts ONE agent (a container), persists its
// UUID, registers with the platform, and routes to peer nodes over TCP/IP.
//
// Config (env — "add the agent to the node" by setting FIPA_AGENT):
//   FIPA_AGENT      df | ams | pa | bs | ba           (which agent to host)
//   FIPA_BIND       0.0.0.0:9000                       (listen address)
//   FIPA_ADVERTISE  <host>:9000                        (address peers use; return addr)
//   FIPA_DATA       /data                              (volume: persisted UUID, PA store)
//   FIPA_AMS        ams-node:9000                       (white pages, for resolution)
//   FIPA_DF         df-node:9000                        (yellow pages)
//   FIPA_UUID       <uuid>                             (pin identity; else mint+persist)
//   FIPA_SEED       {"ledger":{...}}                   (PA ledger seed)
//   FIPA_KICK       2                                  (BA: seconds before kickoff)
//   FIPA_BOOT_DELAY 1                                  (seconds before registering)

use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use fipa_wasm_agents::identity::{AgentId, Header};
use fipa_wasm_agents::process::Node;
use fipa_wasm_agents::wasm::{AgentRuntime, NativeRuntime};
use unl_agent::{Agent, Ctx};
use uuid::Uuid;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}
fn env_or(k: &str, d: &str) -> String {
    env(k).unwrap_or_else(|| d.into())
}

/// Build the hosted agent and the service it provides (for DF registration).
fn build_agent(name: &str, data: &str) -> (Box<dyn AgentRuntime + Send>, Option<String>) {
    match name {
        "df" => (Box::new(NativeRuntime::new(df_agent::Df::new())), None),
        "ams" => (Box::new(NativeRuntime::new(ams_agent::Ams::new())), None),
        "pa" => {
            let mut pa = pa_agent::Pa::open(format!("{data}/pa")).expect("open PA store");
            if let Some(seed) = env("FIPA_SEED") {
                pa.on_seed(seed.as_bytes(), &mut Ctx::new());
            }
            (Box::new(NativeRuntime::new(pa)), None)
        }
        "bs" => (Box::new(NativeRuntime::new(bs_agent::Seller::new(true))), Some("bookselling".into())),
        "ba" => (Box::new(NativeRuntime::new(ba_agent::Buyer::new())), None),
        other => panic!("unknown FIPA_AGENT '{other}' (df|ams|pa|bs|ba)"),
    }
}

fn main() {
    let name = env("FIPA_AGENT").expect("set FIPA_AGENT (df|ams|pa|bs|ba)");
    let bind = env_or("FIPA_BIND", "0.0.0.0:9000");
    let advertise = env_or("FIPA_ADVERTISE", &bind);
    let data = env_or("FIPA_DATA", "/data");
    std::fs::create_dir_all(&data).ok();

    // Identity: pin via FIPA_UUID, else mint at spawn and persist (survives reboot).
    let uuid = match env("FIPA_UUID") {
        Some(u) => u,
        None => {
            let header =
                Header { type_id: Uuid::new_v4(), desc: format!("{name} service"), name: Some(name.clone()) };
            AgentId::load_or_mint(&header, format!("{data}/id")).expect("persist id").id()
        }
    };

    let (agent, service) = build_agent(&name, &data);
    let mut node = Node::new(&uuid, &name, &advertise, agent);
    // Persisted node identities: Ed25519 signing key (R1) + Noise static key (R2).
    let _ = node.load_key(format!("{data}/node_key"));
    let _ = node.load_noise(format!("{data}/noise_key"));
    if let Some(a) = env("FIPA_AMS") {
        node.add_route("ams", &a);
        node.set_ams(&a);
    }
    if let Some(a) = env("FIPA_DF") {
        node.add_route("df", &a);
    }
    if let Some(a) = env("FIPA_PA") {
        node.add_route("pa", &a);
    }
    let (tx, rx) = mpsc::channel();
    node.set_sink(tx);

    let listener = TcpListener::bind(&bind).expect("bind FIPA_BIND");
    let short = &uuid[..8.min(uuid.len())];
    eprintln!("[{name} {short}] listening {bind}, advertise {advertise}");

    // Let the well-known nodes (ams/df) come up, then register: bind UUID→address
    // with AMS, offer the service to DF.
    let boot: u64 = env_or("FIPA_BOOT_DELAY", "1").parse().unwrap_or(1);
    thread::sleep(Duration::from_secs(boot));
    node.register(service.as_deref());
    eprintln!("[{name} {short}] registered with the platform");

    // The buyer self-kicks via a local channel (trusted, in-process — never the wire).
    let kick_tx = if name == "ba" {
        let (tx, rx) = mpsc::channel();
        node.set_kick(rx);
        Some(tx)
    } else {
        None
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    let sd = shutdown.clone();
    let handle = thread::spawn(move || node.serve(listener, sd));

    // The buyer kicks itself off once discovery is ready.
    if let Some(tx) = kick_tx {
        let delay: u64 = env_or("FIPA_KICK", "2").parse().unwrap_or(2);
        thread::sleep(Duration::from_secs(delay));
        eprintln!("[{name} {short}] starting the purchase");
        let _ = tx.send((b"obj(start, buy)".to_vec(), Vec::new()));
    }

    // Surface undeliverable messages (the buyer's verdict lands here as "result").
    for msg in rx {
        println!(
            "[{name}] RESULT → '{}' : {} {}",
            msg.to,
            String::from_utf8_lossy(&msg.unl),
            String::from_utf8_lossy(&msg.body)
        );
    }
    let _ = handle.join();
}
