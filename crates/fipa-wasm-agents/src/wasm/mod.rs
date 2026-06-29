// wasm/mod.rs - WASM Runtime Module

//! WASM Runtime for FIPA agents.
//!
//! This module provides the wasmtime-based runtime for executing
//! WASM agents with the FIPA component model interface.

mod agent_runtime;
mod host;
mod runtime;
mod wasmi_engine;

pub use agent_runtime::{AgentRuntime, NativeRuntime};
pub use host::{HostState, OutboundIntent};
pub use runtime::{WasmRuntime, WasmtimeEngine};
pub use wasmi_engine::{WasmiEngine, WasmiModule};
