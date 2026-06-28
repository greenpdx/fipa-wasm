// agent_host.rs - One agent in its own process (the "host" isolation profile).
//
// Applies OS resource caps (setrlimit) BEFORE loading the agent, then connects
// to the node over a Unix domain socket and serves the agent. A wasm agent is
// loaded from a bundle; a native agent is one compiled in here (where DF/AMS/PA
// will register). The node SIGKILLs this process to kill the agent.
//
//   agent-host --socket <path> (--wasm <bundle> | --native <name>)
//              [--mem-bytes N] [--cpu-secs N]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use fipa_wasm_agents::content::block::{BlockFile, TAG_WASM};
use fipa_wasm_agents::process::{apply_limits, serve, Limits};
use fipa_wasm_agents::proto;
use fipa_wasm_agents::wasm::{AgentRuntime, NativeRuntime, WasmRuntime};

/// A sample native agent — echoes each message back to "peer". Stands in for the
/// infrastructure agents (DF/AMS/PA) that will register here.
struct Echo;
impl unl_agent::Agent for Echo {
    fn on_message(&mut self, unl: &str, body: &[u8], ctx: &mut unl_agent::Ctx) {
        ctx.send("peer", unl, body.to_vec());
    }
}

/// Registry of native agents compiled into the host.
fn native(name: &str) -> Option<Box<dyn AgentRuntime>> {
    match name {
        "echo" => Some(Box::new(NativeRuntime::new(Echo))),
        _ => None,
    }
}

fn build_wasm(path: &PathBuf) -> Result<Box<dyn AgentRuntime>> {
    let bytes = std::fs::read(path)?;
    let wasm = if BlockFile::is_block_container(&bytes) {
        BlockFile::decode(&bytes)?
            .get(TAG_WASM)
            .ok_or_else(|| anyhow!("bundle has no WASM block"))?
            .to_vec()
    } else {
        bytes
    };
    let caps = proto::AgentCapabilities {
        max_execution_time_ms: 1000,
        max_memory_bytes: 64 * 1024 * 1024,
        storage_quota_bytes: 1024 * 1024,
        ..Default::default()
    };
    Ok(Box::new(WasmRuntime::new(&wasm, &caps)?))
}

fn main() -> Result<()> {
    let mut socket: Option<String> = None;
    let mut wasm: Option<String> = None;
    let mut native_name: Option<String> = None;
    let mut mem_bytes = 0u64;
    let mut cpu_secs = 0u64;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket = args.next(),
            "--wasm" => wasm = args.next(),
            "--native" => native_name = args.next(),
            "--mem-bytes" => mem_bytes = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--cpu-secs" => cpu_secs = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            other => return Err(anyhow!("unknown arg: {other}")),
        }
    }
    let socket = socket.ok_or_else(|| anyhow!("--socket required"))?;

    // Cap resources BEFORE loading the agent, so it can never run unbounded.
    apply_limits(&Limits { mem_bytes, cpu_secs });

    let runtime = match (native_name, wasm) {
        (Some(name), _) => native(&name).ok_or_else(|| anyhow!("unknown native agent: {name}"))?,
        (None, Some(path)) => build_wasm(&PathBuf::from(path))?,
        (None, None) => return Err(anyhow!("one of --native / --wasm required")),
    };

    let stream = UnixStream::connect(&socket)?;
    serve(runtime, stream)
}
