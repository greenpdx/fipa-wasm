//! The **wasmi** backend (IoT profile) for the Engine seam (E2).
//!
//! A small interpreter — no JIT — implementing the same [`crate::adapters::Engine`]
//! / [`crate::adapters::EngineModule`] traits as the wasmtime [`super::WasmRuntime`].
//! So the *same agent ABI* runs on a constrained node: the node selects the engine
//! by profile, the agent code is unchanged.
//!
//! NB: wasmi consumes **binary** wasm (it does not parse WAT). CPU is bounded by
//! fuel; a hard memory cap is a follow-up (wasmi modules are small on IoT).

use anyhow::{anyhow, Result};
use wasmi::{Caller, Config, Engine as WasmiCore, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Val};

use crate::adapters::{EngineModule, HostHooks, Limits};
use crate::wasm::{AgentRuntime, OutboundIntent};

/// An instantiated agent module on the wasmi interpreter.
pub struct WasmiModule {
    store: Store<StoreLimits>,
    instance: wasmi::Instance,
    hooks: HostHooks,
    fuel: u64, // per-call CPU budget
}

/// The wasmi backend (IoT). Implements the Engine seam alongside `WasmtimeEngine`.
pub struct WasmiEngine;

impl crate::adapters::Engine for WasmiEngine {
    type Module = WasmiModule;

    fn instantiate(&self, code: &[u8], limits: Limits, hooks: HostHooks) -> Result<WasmiModule> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = WasmiCore::new(&config);
        let module = Module::new(&engine, code).map_err(|e| anyhow!("wasmi compile: {e}"))?;
        // Bound linear-memory growth (audit H6): wasmi otherwise honours only fuel,
        // so a guest could `memory.grow` until the host is OOM-killed.
        let limiter = StoreLimitsBuilder::new().memory_size(limits.mem_bytes).build();
        let mut store = Store::new(&engine, limiter);
        store.set_fuel(limits.fuel).ok();
        store.limiter(|lim| lim);

        let mut linker = <Linker<StoreLimits>>::new(&engine);
        let sends = hooks.sends.clone();
        linker
            .func_wrap(
                "fipa:agent/messaging",
                "send-unl",
                move |caller: Caller<StoreLimits>, rp: i32, rl: i32, up: i32, ul: i32, bp: i32, bl: i32| {
                    let Some(Extern::Memory(mem)) = caller.get_export("memory") else { return };
                    let data = mem.data(&caller);
                    let slice = |ptr: i32, len: i32| -> Vec<u8> {
                        let s = ptr as usize;
                        // saturating add: a negative/huge ptr or len can never overflow
                        // into a panic or an out-of-range slice (audit M10).
                        data.get(s..s.saturating_add(len as usize)).map(<[u8]>::to_vec).unwrap_or_default()
                    };
                    let receiver = String::from_utf8_lossy(&slice(rp, rl)).into_owned();
                    let unl = slice(up, ul);
                    let body = slice(bp, bl);
                    // M3 — bound guest egress (see crate::adapters egress caps).
                    if unl.len() + body.len() > crate::adapters::MAX_SEND_BYTES {
                        return;
                    }
                    let mut guard = sends.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.len() >= crate::adapters::MAX_QUEUED_SENDS {
                        return;
                    }
                    guard.push(OutboundIntent { receiver, unl, body });
                },
            )
            .map_err(|e| anyhow!("wasmi link: {e}"))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| anyhow!("wasmi instantiate: {e}"))?
            .start(&mut store)
            .map_err(|e| anyhow!("wasmi start: {e}"))?;
        Ok(WasmiModule { store, instance, hooks, fuel: limits.fuel.max(1) })
    }
}

// Drive a wasmi agent through the node's `AgentRuntime` seam — so a WasmiModule
// mounts in a Node exactly like the wasmtime WasmRuntime (E2 integration).
impl AgentRuntime for WasmiModule {
    fn init(&mut self) -> Result<()> {
        let fuel = self.fuel;
        self.refuel(fuel);
        self.call_void("init")
    }

    fn config(&mut self, from: &str, unl: &[u8], body: &[u8]) -> Result<()> {
        let fuel = self.fuel;
        self.refuel(fuel);
        if self.call_io("deliver", &[from.as_bytes(), unl, body])? {
            Ok(())
        } else {
            self.refuel(fuel);
            self.call_io("config", &[unl, body]).map(|_| ())
        }
    }

    fn take_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut *self.hooks.sends.lock().unwrap())
    }

    fn run(&mut self) -> Result<bool> {
        let fuel = self.fuel;
        self.refuel(fuel);
        Ok(self.call_i32("run")? != 0)
    }

    fn shutdown(&mut self) -> Result<()> {
        let fuel = self.fuel;
        self.refuel(fuel);
        self.call_void("shutdown")
    }

    fn snapshot(&mut self) -> Vec<u8> {
        let fuel = self.fuel;
        self.refuel(fuel);
        self.call_packed("snapshot").unwrap_or_default()
    }

    fn restore(&mut self, state: &[u8]) {
        let fuel = self.fuel;
        self.refuel(fuel);
        let _ = self.call_io("restore", &[state]);
    }
}

impl WasmiModule {
    fn guest_alloc(&mut self, n: usize) -> Result<i32> {
        let f = self
            .instance
            .get_typed_func::<i32, i32>(&self.store, "alloc")
            .map_err(|_| anyhow!("alloc not found"))?;
        Ok(f.call(&mut self.store, n as i32)?)
    }

    fn write_bytes(&mut self, ptr: i32, bytes: &[u8]) -> Result<()> {
        let mem = self.instance.get_memory(&self.store, "memory").ok_or_else(|| anyhow!("no memory"))?;
        mem.write(&mut self.store, ptr as usize, bytes).map_err(|e| anyhow!("mem write: {e}"))
    }

    /// Drain the messages the agent emitted via `send-unl` (the node's sink).
    pub fn take_sends(&mut self) -> Vec<OutboundIntent> {
        std::mem::take(&mut *self.hooks.sends.lock().unwrap())
    }
}

impl EngineModule for WasmiModule {
    fn refuel(&mut self, fuel: u64) {
        let _ = self.store.set_fuel(fuel);
    }

    fn call_void(&mut self, func: &str) -> Result<()> {
        let f = self
            .instance
            .get_typed_func::<(), ()>(&self.store, func)
            .map_err(|_| anyhow!("{func} not found"))?;
        f.call(&mut self.store, ())?;
        Ok(())
    }

    fn call_i32(&mut self, func: &str) -> Result<i32> {
        let f = self
            .instance
            .get_typed_func::<(), i32>(&self.store, func)
            .map_err(|_| anyhow!("{func} not found"))?;
        Ok(f.call(&mut self.store, ())?)
    }

    fn call_io(&mut self, func: &str, args: &[&[u8]]) -> Result<bool> {
        let Some(f) = self.instance.get_func(&self.store, func) else {
            return Ok(false);
        };
        let mut params = Vec::with_capacity(args.len() * 2);
        for a in args {
            let ptr = self.guest_alloc(a.len())?;
            self.write_bytes(ptr, a)?;
            params.push(Val::I32(ptr));
            params.push(Val::I32(a.len() as i32));
        }
        f.call(&mut self.store, &params, &mut []).map_err(|e| anyhow!("wasmi call {func}: {e}"))?;
        Ok(true)
    }

    fn call_packed(&mut self, func: &str) -> Result<Vec<u8>> {
        let f = match self.instance.get_typed_func::<(), i64>(&self.store, func) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let packed = f.call(&mut self.store, ())?;
        let ptr = (packed >> 32) as usize;
        let len = (packed & 0xffff_ffff) as usize;
        let Some(mem) = self.instance.get_memory(&self.store, "memory") else {
            return Ok(Vec::new());
        };
        let data = mem.data(&self.store);
        Ok(data.get(ptr..ptr.saturating_add(len)).map(<[u8]>::to_vec).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::Engine;

    const COUNTER: &str = r#"
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

    fn limits() -> Limits {
        Limits { fuel: 100_000_000, mem_bytes: 64 * 1024 * 1024 }
    }

    #[test]
    fn wasmi_runs_an_agent_through_the_engine_seam() {
        let wasm = wat::parse_str(COUNTER).unwrap(); // wasmi needs binary wasm
        let mut a = WasmiEngine.instantiate(&wasm, limits(), HostHooks::default()).unwrap();
        a.refuel(limits().fuel);
        a.call_void("init").unwrap();
        for _ in 0..3 {
            a.refuel(limits().fuel);
            a.call_io("deliver", &[b"x", b"y", b"z"]).unwrap();
        }
        a.refuel(limits().fuel);
        let snap = a.call_packed("snapshot").unwrap();
        assert_eq!(snap, vec![3, 0, 0, 0]); // n = 3, same as the wasmtime backend

        // a fresh wasmi instance restores the captured state
        let mut b = WasmiEngine.instantiate(&wasm, limits(), HostHooks::default()).unwrap();
        b.refuel(limits().fuel);
        b.call_io("restore", &[&snap]).unwrap();
        b.refuel(limits().fuel);
        assert_eq!(b.call_packed("snapshot").unwrap(), vec![3, 0, 0, 0]);
    }

    #[test]
    fn wasmi_caps_linear_memory() {
        // A module declaring 4 pages (256 KiB) cannot instantiate under a 64 KiB cap
        // (H6 — wasmi otherwise ignores the memory limit).
        let wat = r#"(module (memory (export "memory") 4) (func (export "init")))"#;
        let wasm = wat::parse_str(wat).unwrap();
        let tight = Limits { fuel: 1_000_000, mem_bytes: 64 * 1024 };
        assert!(WasmiEngine.instantiate(&wasm, tight, HostHooks::default()).is_err());
        // the same module fits under a generous cap
        let roomy = Limits { fuel: 1_000_000, mem_bytes: 1024 * 1024 };
        assert!(WasmiEngine.instantiate(&wasm, roomy, HostHooks::default()).is_ok());
    }
}
