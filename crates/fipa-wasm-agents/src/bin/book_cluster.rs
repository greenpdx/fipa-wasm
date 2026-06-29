// book_cluster.rs — the full book-buy across 5 separate nodes over TCP/IP.
//
// Each agent runs in its own Node (its own TCP address) on 127.0.0.1 — the exact
// same protocol the Docker deployment uses over container IPs. Proves cross-node
// discovery (DF), name resolution (AMS), authenticated UUID routing, and the
// escrow purchase, all over the wire.

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use fipa_wasm_agents::identity::{AgentId, Header};
use fipa_wasm_agents::process::{send_message, Node};
use fipa_wasm_agents::wasm::NativeRuntime;
use uuid::Uuid;

fn aid(name: &str) -> AgentId {
    let h = Header { type_id: Uuid::new_v4(), desc: name.into(), name: Some(name.into()) };
    AgentId::spawn(&h)
}

fn bound() -> (TcpListener, String) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    (l, a)
}

fn platform(n: &mut Node, ams: &str, df: &str, pa: &str) {
    n.add_route("ams", ams);
    n.add_route("df", df);
    n.add_route("pa", pa);
    n.set_ams(ams);
}

fn main() {
    println!("=== book-buy across 5 TCP nodes (loopback; identical protocol to Docker over IP) ===");

    let (df, ams, pa, bs, ba) = (aid("df"), aid("ams"), aid("pa"), aid("bookSeller"), aid("BA"));
    let (ldf, df_a) = bound();
    let (lams, ams_a) = bound();
    let (lpa, pa_a) = bound();
    let (lbs, bs_a) = bound();
    let (lba, ba_a) = bound();
    println!("  df={df_a}  ams={ams_a}  pa={pa_a}  bookSeller={bs_a}  BA={ba_a}");

    let shutdown = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();
    macro_rules! serve {
        ($node:expr, $listener:expr) => {{
            let sd = shutdown.clone();
            let mut n = $node;
            handles.push(thread::spawn(move || n.serve($listener, sd)));
        }};
    }

    // White & yellow pages come up first so others can register against them.
    let ams_node = Node::new(&ams.id(), "ams", &ams_a, Box::new(NativeRuntime::new(ams_agent::Ams::new())));
    serve!(ams_node, lams);
    let df_node = Node::new(&df.id(), "df", &df_a, Box::new(NativeRuntime::new(df_agent::Df::new())));
    serve!(df_node, ldf);
    thread::sleep(Duration::from_millis(100));

    // PA (escrow), funded for the buyer.
    let mut pa_store =
        pa_agent::Pa::open(std::env::temp_dir().join(format!("cluster-pa-{}", std::process::id()))).unwrap();
    pa_store.credit(ba.id(), 10000);
    let mut pa_node = Node::new(&pa.id(), "pa", &pa_a, Box::new(NativeRuntime::new(pa_store)));
    platform(&mut pa_node, &ams_a, &df_a, &pa_a);
    serve!(pa_node, lpa);

    // Seller — registers its service with DF and its address with AMS.
    let mut bs_node = Node::new(&bs.id(), "bookSeller", &bs_a, Box::new(NativeRuntime::new(bs_agent::Seller::new(true))));
    platform(&mut bs_node, &ams_a, &df_a, &pa_a);
    bs_node.register(Some("bookselling"));
    serve!(bs_node, lbs);

    // Buyer — its verdict is surfaced through the sink.
    let (tx, rx) = mpsc::channel();
    let mut ba_node = Node::new(&ba.id(), "BA", &ba_a, Box::new(NativeRuntime::new(ba_agent::Buyer::new())));
    platform(&mut ba_node, &ams_a, &df_a, &pa_a);
    ba_node.set_sink(tx);
    let ba_kick = ba_node.sealed_kick(b"obj(start, buy)", b""); // authenticated self-kick
    serve!(ba_node, lba);

    thread::sleep(Duration::from_millis(300)); // registrations settle

    println!("  → kicking off the buyer\n");
    let _ = send_message(&ba_a, &ba_kick);

    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(m) => println!("RESULT: {} {}", String::from_utf8_lossy(&m.unl), String::from_utf8_lossy(&m.body)),
        Err(_) => println!("RESULT: timed out — no verdict"),
    }
    shutdown.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }
}
