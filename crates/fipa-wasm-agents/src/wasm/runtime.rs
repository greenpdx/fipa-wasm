// wasm/runtime.rs - Wasmtime Component Model Runtime

use anyhow::{anyhow, Result};
use wasmtime::*;

use crate::adapters::{EngineModule, HostHooks, Limits};
use crate::proto;
use super::host::{HostState, OutboundIntent};

/// WASM Runtime for executing agent modules
pub struct WasmRuntime {
    /// Wasmtime engine
    #[allow(dead_code)]
    engine: Engine,

    /// Compiled module
    #[allow(dead_code)]
    module: Module,

    /// Module bytecode (for migration)
    module_bytes: Vec<u8>,

    /// Store with host state
    store: Store<HostState>,

    /// Instance of the module
    instance: Instance,

    /// Agent capabilities
    capabilities: proto::AgentCapabilities,

    /// The host import table (the `send-unl` sink the node drains) — Engine seam.
    hooks: HostHooks,
}

impl WasmRuntime {
    /// Create a new runtime from WASM bytecode
    pub fn new(wasm_bytes: &[u8], capabilities: &proto::AgentCapabilities) -> Result<Self> {
        Self::build(wasm_bytes, capabilities.clone(), HostHooks::default())
    }

    /// Instantiate with an explicit host import table — the Engine seam's path.
    fn build(code: &[u8], capabilities: proto::AgentCapabilities, hooks: HostHooks) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(false);
        config.consume_fuel(true);
        // CPU is bounded by fuel: every guest instruction costs fuel and a runaway
        // loop traps when the per-call budget is exhausted. A wall-clock epoch
        // interrupt (audit L3) would only add value once a *blocking* host import
        // exists (none do today — all host calls return immediately), and it needs a
        // shared-engine watchdog; deferred until that architecture lands.
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, code)?;
        let host_state = HostState::new(capabilities.clone());
        let mut store = Store::new(&engine, host_state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(capabilities.max_execution_time_ms.max(1) as u64 * 1_000_000)?;
        let mut linker = Linker::new(&engine);
        Self::define_host_functions(&mut linker, &hooks)?;
        let instance = linker.instantiate(&mut store, &module)?;
        Ok(Self { engine, module, module_bytes: code.to_vec(), store, instance, capabilities, hooks })
    }

    /// Define host functions in the linker
    fn define_host_functions(linker: &mut Linker<HostState>, hooks: &HostHooks) -> Result<()> {
        // Messaging functions
        linker.func_wrap("fipa:agent/messaging", "send-message", |mut caller: Caller<'_, HostState>, _msg_ptr: i32, _msg_len: i32| -> i32 {
            // Implementation would extract message from WASM memory and send
            let state = caller.data_mut();
            state.messages_sent += 1;
            0 // Success
        })?;

        linker.func_wrap("fipa:agent/messaging", "receive-message", |mut caller: Caller<'_, HostState>| -> i64 {
            let state = caller.data_mut();
            if let Some(_msg) = state.mailbox.pop_front() {
                // Return pointer to message in WASM memory
                1 // Has message
            } else {
                0 // No message
            }
        })?;

        linker.func_wrap("fipa:agent/messaging", "has-messages", |caller: Caller<'_, HostState>| -> i32 {
            let state = caller.data();
            if state.mailbox.is_empty() { 0 } else { 1 }
        })?;

        // The agent emits a message: send-unl(receiver, unl, body) as (ptr,len)
        // triples into WASM memory. The node validates + packages + transmits it.
        let sends = hooks.sends.clone();
        linker.func_wrap(
            "fipa:agent/messaging",
            "send-unl",
            move |mut caller: Caller<'_, HostState>,
             rp: i32, rl: i32, up: i32, ul: i32, bp: i32, bl: i32| {
                let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                else {
                    return;
                };
                let read = |caller: &Caller<'_, HostState>, ptr: i32, len: i32| -> Vec<u8> {
                    let data = memory.data(caller);
                    let start = ptr as usize;
                    // saturating add: a negative/huge ptr or len can never overflow
                    // into a panic or an out-of-range slice (audit M10).
                    data.get(start..start.saturating_add(len as usize))
                        .map(<[u8]>::to_vec)
                        .unwrap_or_default()
                };
                let receiver = String::from_utf8_lossy(&read(&caller, rp, rl)).into_owned();
                let unl = read(&caller, up, ul);
                let body = read(&caller, bp, bl);
                // M3 — bound a guest's egress: a single message can't exceed 1 MiB and a
                // single call can't queue more than MAX_QUEUED_SENDS intents, so a guest
                // cannot amplify host-heap use beyond its own (capped) memory.
                if unl.len() + body.len() > crate::adapters::MAX_SEND_BYTES {
                    return;
                }
                let mut guard = sends.lock().unwrap_or_else(|e| e.into_inner());
                if guard.len() >= crate::adapters::MAX_QUEUED_SENDS {
                    return;
                }
                crate::flow!(
                    "wasm: ← agent emitted send-unl → '{}' (unl={} bytes, body={} bytes)",
                    receiver,
                    unl.len(),
                    body.len()
                );
                guard.push(OutboundIntent { receiver, unl, body });
            },
        )?;

        // Lifecycle functions
        linker.func_wrap("fipa:agent/lifecycle", "get-agent-id", |_caller: Caller<'_, HostState>| -> i64 {
            // Return pointer to agent ID string
            0
        })?;

        linker.func_wrap("fipa:agent/lifecycle", "request-shutdown", |mut caller: Caller<'_, HostState>| {
            caller.data_mut().shutdown_requested = true;
        })?;

        linker.func_wrap("fipa:agent/lifecycle", "is-shutdown-requested", |caller: Caller<'_, HostState>| -> i32 {
            if caller.data().shutdown_requested { 1 } else { 0 }
        })?;

        // Logging functions
        linker.func_wrap("fipa:agent/logging", "log", |mut caller: Caller<'_, HostState>, _level: i32, _msg_ptr: i32, _msg_len: i32| {
            // Read message from WASM memory and log
            let state = caller.data_mut();
            state.log_count += 1;
        })?;

        // Storage functions
        linker.func_wrap("fipa:agent/storage", "store", |_caller: Caller<'_, HostState>, _key_ptr: i32, _key_len: i32, _val_ptr: i32, _val_len: i32| -> i32 {
            // Store data
            0 // Success
        })?;

        linker.func_wrap("fipa:agent/storage", "load", |_caller: Caller<'_, HostState>, _key_ptr: i32, _key_len: i32| -> i64 {
            // Load data, return pointer
            0
        })?;

        // Timing functions
        linker.func_wrap("fipa:agent/timing", "now", |_caller: Caller<'_, HostState>| -> i64 {
            chrono::Utc::now().timestamp_millis()
        })?;

        linker.func_wrap("fipa:agent/timing", "monotonic-now", |_caller: Caller<'_, HostState>| -> i64 {
            std::time::Instant::now().elapsed().as_nanos() as i64
        })?;

        // Migration functions
        linker.func_wrap("fipa:agent/migration", "get-current-node", |_caller: Caller<'_, HostState>| -> i64 {
            // Return pointer to node ID
            0
        })?;

        linker.func_wrap("fipa:agent/migration", "is-migrating", |caller: Caller<'_, HostState>| -> i32 {
            if caller.data().is_migrating { 1 } else { 0 }
        })?;

        Ok(())
    }

    /// Per-call CPU budget (wasmtime fuel). Each agent entry point gets a *fresh*
    /// budget, so a looping or runaway agent traps (out of fuel) and is contained,
    /// while a well-behaved agent gets the full budget on every message (H3/R7).
    fn call_fuel(&self) -> u64 {
        (self.capabilities.max_execution_time_ms.max(1) as u64).saturating_mul(1_000_000)
    }

    /// Call the agent's init function (via the Engine seam).
    pub fn call_init(&mut self) -> Result<()> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        self.call_void("init")
    }

    /// Call the agent's run function.
    pub fn call_run(&mut self) -> Result<bool> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        Ok(self.call_i32("run")? != 0)
    }

    /// Call the agent's shutdown function.
    pub fn call_shutdown(&mut self) -> Result<()> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        self.call_void("shutdown")
    }

    /// Handle an incoming message
    pub fn handle_message(&mut self, msg: &proto::AclMessage) -> Result<bool> {
        // Add message to mailbox for WASM to retrieve
        self.store.data_mut().mailbox.push_back(msg.clone());

        // Call handle-message if it exists
        if let Ok(handle_msg) = self.instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, "handle-message")
        {
            // Would pass message pointer and length
            let result = handle_msg.call(&mut self.store, (0, 0))?;
            Ok(result != 0)
        } else {
            // No handle-message export, will be processed in run()
            Ok(false)
        }
    }

    /// Deliver decoded content to the agent: write the UNL bytes and the body
    /// into WASM memory (via the guest's `alloc`) and call its
    /// `config(unl_ptr, unl_len, body_ptr, body_len)` export. The agent keeps
    /// its state across calls (its memory persists), so `config` runs once at
    /// startup to seed state and again per inbound message. A guest without a
    /// `config` export is a graceful no-op.
    pub fn call_config(&mut self, unl: &[u8], body: &[u8]) -> Result<()> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        crate::flow!("wasm: → config(unl={} bytes, body={} bytes)", unl.len(), body.len());
        self.call_io("config", &[unl, body])?; // Ok(false) if absent → graceful no-op
        Ok(())
    }

    /// Deliver `(unl, body)` from sender `from` via the agent's `deliver` export
    /// (from-aware). Returns `Ok(false)` if the guest has no `deliver` export, so
    /// the caller can fall back to `call_config`.
    pub fn call_deliver(&mut self, from: &[u8], unl: &[u8], body: &[u8]) -> Result<bool> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        crate::flow!("wasm: → deliver(from={} bytes, unl={} bytes)", from.len(), unl.len());
        self.call_io("deliver", &[from, unl, body])
    }

    /// Drain the UNL send intents the agent emitted via `send-unl`. The node
    /// validates each against the receiver's vocabulary, packages it, and
    /// transmits it.
    pub fn take_unl_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut *self.hooks.sends.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Capture the agent's state via its `snapshot` export (state-based migration).
    /// Empty if the guest exports no `snapshot` (a stateless agent).
    pub fn call_snapshot(&mut self) -> Vec<u8> {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        self.call_packed("snapshot").unwrap_or_default()
    }

    /// Restore state captured by [`Self::call_snapshot`] via the `restore` export.
    pub fn call_restore(&mut self, state: &[u8]) {
        let fuel = self.call_fuel();
        self.refuel(fuel);
        let _ = self.call_io("restore", &[state]);
    }

    /// Allocate `n` bytes in WASM memory via the guest's `alloc` export.
    fn guest_alloc(&mut self, n: usize) -> Result<i32> {
        let alloc = self
            .instance
            .get_typed_func::<i32, i32>(&mut self.store, "alloc")
            .map_err(|_| anyhow!("agent exports `config` but not `alloc`"))?;
        Ok(alloc.call(&mut self.store, n as i32)?)
    }

    /// Copy `bytes` into WASM memory at `ptr`.
    fn write_bytes(&mut self, ptr: i32, bytes: &[u8]) -> Result<()> {
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| anyhow!("memory export not found"))?;
        let start = ptr as usize;
        let end = start
            .checked_add(bytes.len())
            .ok_or_else(|| anyhow!("config write overflow"))?;
        let data = memory.data_mut(&mut self.store);
        let dst = data
            .get_mut(start..end)
            .ok_or_else(|| anyhow!("config write out of bounds"))?;
        dst.copy_from_slice(bytes);
        Ok(())
    }

    /// Capture agent state for migration
    pub fn capture_state(&mut self) -> Result<proto::AgentState> {
        // Get memory
        let memory = self.instance
            .get_memory(&mut self.store.as_context_mut(), "memory")
            .ok_or_else(|| anyhow!("memory not found"))?;

        let memory_data = memory.data(&self.store);

        // Get globals
        let globals = Vec::new();
        // Would iterate over exports to find globals

        Ok(proto::AgentState {
            memory: memory_data.to_vec(),
            globals,
            conversations: vec![],
            storage: self.store.data().storage.clone(),
            custom_data: vec![],
        })
    }

    /// Restore agent state after migration
    pub fn restore_state(&mut self, state: &proto::AgentState) -> Result<()> {
        // Restore memory
        if !state.memory.is_empty() {
            let memory = self.instance
                .get_memory(&mut self.store.as_context_mut(), "memory")
                .ok_or_else(|| anyhow!("memory not found"))?;

            let min_len = state.memory.len().min(memory.data_size(&self.store));
            memory.data_mut(&mut self.store.as_context_mut())[..min_len]
                .copy_from_slice(&state.memory[..min_len]);
        }

        // Restore storage
        self.store.data_mut().storage = state.storage.clone();

        Ok(())
    }

    /// Get module bytes
    pub fn get_module_bytes(&self) -> &[u8] {
        &self.module_bytes
    }

    /// Get current memory size
    pub fn memory_size(&mut self) -> usize {
        self.instance
            .get_memory(&mut self.store.as_context_mut(), "memory")
            .map(|m| m.data_size(&self.store))
            .unwrap_or(0)
    }
}

// The wasmtime backend's implementation of the Engine seam: the five mechanical
// ops every wasm engine (wasmtime today; wasmi/browser next) must provide.
impl EngineModule for WasmRuntime {
    fn refuel(&mut self, fuel: u64) {
        let _ = self.store.set_fuel(fuel);
    }

    fn call_void(&mut self, func: &str) -> Result<()> {
        let f = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, func)
            .map_err(|_| anyhow!("{func} not found"))?;
        f.call(&mut self.store, ())?;
        Ok(())
    }

    fn call_i32(&mut self, func: &str) -> Result<i32> {
        let f = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, func)
            .map_err(|_| anyhow!("{func} not found"))?;
        Ok(f.call(&mut self.store, ())?)
    }

    fn call_io(&mut self, func: &str, args: &[&[u8]]) -> Result<bool> {
        let Some(f) = self.instance.get_func(&mut self.store, func) else {
            return Ok(false);
        };
        let mut params = Vec::with_capacity(args.len() * 2);
        for a in args {
            let ptr = self.guest_alloc(a.len())?;
            self.write_bytes(ptr, a)?;
            params.push(Val::I32(ptr));
            params.push(Val::I32(a.len() as i32));
        }
        f.call(&mut self.store, &params, &mut [])?;
        Ok(true)
    }

    fn call_packed(&mut self, func: &str) -> Result<Vec<u8>> {
        let f = match self.instance.get_typed_func::<(), i64>(&mut self.store, func) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let packed = f.call(&mut self.store, ())?;
        let ptr = (packed >> 32) as usize;
        let len = (packed & 0xffff_ffff) as usize;
        let Some(memory) = self.instance.get_memory(&mut self.store, "memory") else {
            return Ok(Vec::new());
        };
        let data = memory.data(&self.store);
        Ok(data.get(ptr..ptr.saturating_add(len)).map(<[u8]>::to_vec).unwrap_or_default())
    }
}

/// The wasmtime backend — the reference [`crate::adapters::Engine`] impl. wasmi
/// (IoT) and browser-WASM implement the same trait against their own engines.
pub struct WasmtimeEngine;

impl crate::adapters::Engine for WasmtimeEngine {
    type Module = WasmRuntime;

    fn instantiate(&self, code: &[u8], limits: Limits, hooks: HostHooks) -> Result<WasmRuntime> {
        let caps = proto::AgentCapabilities {
            max_execution_time_ms: (limits.fuel / 1_000_000).max(1),
            max_memory_bytes: limits.mem_bytes as u64,
            ..Default::default()
        };
        WasmRuntime::build(code, caps, hooks)
    }
}

impl std::fmt::Debug for WasmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmRuntime")
            .field("module_size", &self.module_bytes.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod config_abi_tests {
    use super::*;

    // A guest that exports memory + a bump `alloc` + `init` + `config`. Its
    // `config` copies the received UNL bytes to offset 0 and the body to offset
    // 512, so the host can observe exactly what crossed the boundary.
    const ECHO_GUEST: &str = r#"
    (module
      (memory (export "memory") 1)
      (global $bump (mut i32) (i32.const 1024))
      (func (export "init"))
      (func (export "alloc") (param $n i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $bump))
        (global.set $bump (i32.add (global.get $bump) (local.get $n)))
        (local.get $p))
      (func (export "config")
        (param $up i32) (param $ul i32) (param $bp i32) (param $bl i32)
        (memory.copy (i32.const 0) (local.get $up) (local.get $ul))
        (memory.copy (i32.const 512) (local.get $bp) (local.get $bl))))
    "#;

    fn caps() -> proto::AgentCapabilities {
        proto::AgentCapabilities { max_execution_time_ms: 1000, ..Default::default() }
    }

    #[test]
    fn call_config_delivers_unl_and_body_into_wasm_memory() {
        let mut rt = WasmRuntime::new(ECHO_GUEST.as_bytes(), &caps()).unwrap();
        rt.call_init().unwrap();
        rt.call_config(b"agt(detect,gate)", &[0x17, 0x2a]).unwrap();

        let mem = rt.instance.get_memory(&mut rt.store, "memory").unwrap();
        let data = mem.data(&rt.store);
        assert_eq!(&data[0..16], b"agt(detect,gate)"); // the UNL the guest received
        assert_eq!(&data[512..514], &[0x17, 0x2a]); // the body the guest received
    }

    // A guest whose `config` never returns (infinite loop) — fuel must trap it.
    const LOOP_GUEST: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "init"))
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "config") (param i32 i32 i32 i32)
        (loop (br 0))))
    "#;

    #[test]
    fn looping_agent_is_capped_by_fuel() {
        // a tiny CPU budget so the trap is fast
        let caps = proto::AgentCapabilities { max_execution_time_ms: 1, ..Default::default() };
        let mut rt = WasmRuntime::new(LOOP_GUEST.as_bytes(), &caps).unwrap();
        rt.call_init().unwrap();
        // the infinite loop runs out of fuel and traps — contained as an Err,
        // never an unbounded hang of the node (H3/R7).
        assert!(rt.call_config(b"x", b"y").is_err());
    }

    // A counter: each `deliver` increments n; `snapshot` returns n (4 bytes LE);
    // `restore` sets n. Exercises state-based migration of a wasm agent.
    const COUNTER_GUEST: &str = r#"
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

    #[test]
    fn wasm_agent_state_snapshots_and_restores() {
        use crate::wasm::AgentRuntime;
        // origin: increment three times, then capture state
        let mut a = WasmRuntime::new(COUNTER_GUEST.as_bytes(), &caps()).unwrap();
        a.call_init().unwrap();
        for _ in 0..3 {
            a.config("x", b"inc", b"").unwrap();
        }
        let snap = a.snapshot();
        assert_eq!(snap, vec![3, 0, 0, 0]); // n = 3 (little-endian i32)

        // destination: a fresh instance restores the captured state
        let mut b = WasmRuntime::new(COUNTER_GUEST.as_bytes(), &caps()).unwrap();
        b.call_init().unwrap();
        b.restore(&snap);
        assert_eq!(b.snapshot(), vec![3, 0, 0, 0]); // state migrated
    }

    #[test]
    fn oversized_memory_is_refused() {
        // a guest demanding 100 pages (6.4 MiB) against a 1 MiB cap won't instantiate
        let caps = proto::AgentCapabilities {
            max_memory_bytes: 1024 * 1024,
            max_execution_time_ms: 1000,
            ..Default::default()
        };
        const BIG: &str = r#"(module (memory (export "memory") 100))"#;
        assert!(WasmRuntime::new(BIG.as_bytes(), &caps).is_err());
    }

    #[test]
    fn call_config_is_a_noop_without_a_config_export() {
        const BARE: &str = r#"(module (memory (export "memory") 1) (func (export "init")))"#;
        let mut rt = WasmRuntime::new(BARE.as_bytes(), &caps()).unwrap();
        rt.call_init().unwrap();
        rt.call_config(b"x", b"y").unwrap(); // graceful no-op
    }

    // A guest that emits one message via the host's `send-unl` import: receiver
    // "bob" at offset 0, UNL "agt(go,bob)" at 16, body byte 0x07 at 64.
    const EMIT_GUEST: &str = r#"
    (module
      (import "fipa:agent/messaging" "send-unl"
        (func $send (param i32 i32 i32 i32 i32 i32)))
      (memory (export "memory") 1)
      (data (i32.const 0) "bob")
      (data (i32.const 16) "agt(go,bob)")
      (data (i32.const 64) "\07")
      (func (export "init"))
      (func (export "run") (result i32)
        (call $send
          (i32.const 0) (i32.const 3)    ;; receiver
          (i32.const 16) (i32.const 11)  ;; unl
          (i32.const 64) (i32.const 1))  ;; body
        (i32.const 0)))
    "#;

    #[test]
    fn send_unl_captures_agent_emit() {
        let mut rt = WasmRuntime::new(EMIT_GUEST.as_bytes(), &caps()).unwrap();
        rt.call_init().unwrap();
        rt.call_run().unwrap();
        let sends = rt.take_unl_sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].receiver, "bob");
        assert_eq!(sends[0].unl, b"agt(go,bob)");
        assert_eq!(sends[0].body, &[0x07]);
        // Drained.
        assert!(rt.take_unl_sends().is_empty());
    }

    // End-to-end: a Rust agent compiled to wasm32 via unl_agent::export_agent!.
    // Skips if the sample agent hasn't been built for wasm32.
    #[test]
    fn rust_wasm_agent_runs_through_the_runtime() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/wasm32-unknown-unknown/debug/greeter_agent.wasm"
        );
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("skip: greeter_agent.wasm not built for wasm32");
            return;
        };
        use crate::wasm::AgentRuntime;
        let mut rt = WasmRuntime::new(&bytes, &caps()).unwrap();
        rt.call_init().unwrap();
        // Via the seam → `deliver`, threading the sender. The greeter replies to
        // ctx.from(), so the receiver must be the authenticated sender.
        rt.config("alice", b"agt(hello, me)", b"ping").unwrap();
        let sends = rt.take_unl_sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].receiver, "alice"); // ctx.from() threaded into wasm
        assert_eq!(sends[0].unl, b"agt(greet, you)");
        assert_eq!(sends[0].body, b"hi from rust-wasm");
    }
}
