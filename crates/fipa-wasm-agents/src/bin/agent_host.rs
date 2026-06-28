// agent_host.rs - One agent in its own process (the "host" isolation profile).
//
// Applies OS resource caps (setrlimit) BEFORE loading the agent, then connects
// to the node over a Unix domain socket and serves it. The agent itself is
// built in-process (this *is* the isolated process); the node SIGKILLs this
// process to kill the agent.
//
//   agent-host --socket <path> (--wasm <bundle> | --native <name>)
//              [--mem-bytes N] [--cpu-secs N]

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Result};
use fipa_wasm_agents::process::{apply_limits, build_runtime, serve, AgentSpec, Limits, Profile};

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

    let spec = match (native_name, wasm) {
        (Some(name), _) => AgentSpec::Native(name),
        (None, Some(path)) => AgentSpec::Wasm(PathBuf::from(path)),
        (None, None) => return Err(anyhow!("one of --native / --wasm required")),
    };

    // Cap resources BEFORE loading the agent, so it can never run unbounded.
    apply_limits(&Limits { mem_bytes, cpu_secs });

    // The agent-host runs the agent in its own process — always in-process here.
    let runtime = build_runtime(&spec, &Profile::InProcess, Path::new(""), Duration::ZERO)?;

    let stream = UnixStream::connect(&socket)?;
    serve(runtime, stream)
}
