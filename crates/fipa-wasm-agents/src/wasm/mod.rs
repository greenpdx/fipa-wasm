// wasm/mod.rs - WASM Runtime Module

//! WASM Runtime for FIPA agents.
//!
//! This module provides the wasmtime-based runtime for executing
//! WASM agents with the FIPA component model interface.

mod runtime;
mod host;

pub use runtime::WasmRuntime;
pub use host::HostState;
