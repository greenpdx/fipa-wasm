// wasm/runtime.rs - Wasmtime Component Model Runtime

use anyhow::{anyhow, Result};
use wasmtime::*;

use crate::proto;
use super::host::HostState;

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
}

impl WasmRuntime {
    /// Create a new runtime from WASM bytecode
    pub fn new(wasm_bytes: &[u8], capabilities: &proto::AgentCapabilities) -> Result<Self> {
        // Configure engine
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(false);
        config.consume_fuel(true);

        let engine = Engine::new(&config)?;

        // Compile module
        let module = Module::new(&engine, wasm_bytes)?;

        // Create host state
        let host_state = HostState::new(capabilities.clone());

        // Create store with fuel limit
        let mut store = Store::new(&engine, host_state);
        store.set_fuel(capabilities.max_execution_time_ms as u64 * 1_000_000)?;

        // Create linker with host functions
        let mut linker = Linker::new(&engine);
        Self::define_host_functions(&mut linker)?;

        // Instantiate module
        let instance = linker.instantiate(&mut store, &module)?;

        Ok(Self {
            engine,
            module,
            module_bytes: wasm_bytes.to_vec(),
            store,
            instance,
            capabilities: capabilities.clone(),
        })
    }

    /// Define host functions in the linker
    fn define_host_functions(linker: &mut Linker<HostState>) -> Result<()> {
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

    /// Call the agent's init function
    pub fn call_init(&mut self) -> Result<()> {
        let init = self.instance
            .get_typed_func::<(), ()>(&mut self.store, "init")
            .map_err(|_| anyhow!("init function not found"))?;

        init.call(&mut self.store, ())?;
        Ok(())
    }

    /// Call the agent's run function
    pub fn call_run(&mut self) -> Result<bool> {
        // Refuel for this tick
        self.store.set_fuel(self.capabilities.max_execution_time_ms as u64 * 1_000)?;

        let run = self.instance
            .get_typed_func::<(), i32>(&mut self.store, "run")
            .map_err(|_| anyhow!("run function not found"))?;

        let result = run.call(&mut self.store, ())?;
        Ok(result != 0)
    }

    /// Call the agent's shutdown function
    pub fn call_shutdown(&mut self) -> Result<()> {
        let shutdown = self.instance
            .get_typed_func::<(), ()>(&mut self.store, "shutdown")
            .map_err(|_| anyhow!("shutdown function not found"))?;

        shutdown.call(&mut self.store, ())?;
        Ok(())
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

impl std::fmt::Debug for WasmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmRuntime")
            .field("module_size", &self.module_bytes.len())
            .finish_non_exhaustive()
    }
}
