// Integration test: a native agent runs in its own agent-host process, driven
// over UDS, and is SIGKILLed on drop.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fipa_wasm_agents::process::{AgentSpec, Limits, ManagedAgent, Profile, ProcessRuntime, Recipe};
use fipa_wasm_agents::wasm::AgentRuntime;

#[test]
fn native_agent_runs_isolated_in_a_process() {
    let host = env!("CARGO_BIN_EXE_agent-host");

    let mut rt = ProcessRuntime::spawn(
        Path::new(host),
        &AgentSpec::Native("echo".into()),
        // mem_bytes 0 = leave RLIMIT_AS unset (a big host binary's virtual map
        // would trip a low cap; real RAM limits want cgroup memory.max). The
        // CPU cap is applied and bounds a runaway loop.
        &Limits { mem_bytes: 0, cpu_secs: 30 },
        Duration::from_secs(10),
    )
    .expect("spawn agent-host");

    rt.init().unwrap();
    rt.config("BA", b"agt(hello, x)", b"ping").unwrap();
    let sends = rt.take_sends();
    assert_eq!(sends.len(), 1, "echo agent should reply once");
    assert_eq!(sends[0].receiver, "peer");
    assert_eq!(sends[0].unl, b"agt(hello, x)");
    assert_eq!(sends[0].body, b"ping");

    // A second message also round-trips through the process.
    rt.config("BA", b"agt(again, y)", b"pong").unwrap();
    let sends = rt.take_sends();
    assert_eq!(sends[0].body, b"pong");

    // Dropping the runtime SIGKILLs the child (verified by no hang/leak here).
    drop(rt);
}

#[test]
fn hosted_agent_restarts_after_a_crash() {
    let host = env!("CARGO_BIN_EXE_agent-host");
    let recipe = Recipe {
        spec: AgentSpec::Native("boomer".into()),
        profile: Profile::Hosted(Limits { mem_bytes: 0, cpu_secs: 30 }),
        host_bin: PathBuf::from(host),
        timeout: Duration::from_secs(10),
        seed_unl: vec![],
        seed_data: vec![],
        max_restarts: 3,
    };
    let mut agent = ManagedAgent::spawn(recipe).expect("spawn managed agent");

    // A normal message echoes.
    let out = agent.deliver("BA", b"agt(hi, x)", b"a").unwrap();
    assert_eq!(out[0].body, b"a");

    // "boom" crashes the child process — deliver fails, but the agent restarts.
    assert!(agent.deliver("BA", b"boom", b"").is_err());
    assert_eq!(agent.restarts(), 1);

    // The restarted agent (a fresh process) works again.
    let out = agent.deliver("BA", b"agt(again, y)", b"b").unwrap();
    assert_eq!(out[0].body, b"b");
}
