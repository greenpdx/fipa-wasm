// platform/mod.rs - FIPA Platform Agents (AMS, DF)
//
//! FIPA Platform Agents
//!
//! This module implements the formal FIPA platform agents:
//! - **AMS (Agent Management System)**: Manages agent lifecycle, naming, and access control
//! - **DF (Directory Facilitator)**: Provides yellow pages service discovery
//!
//! # Example
//!
//! ```ignore
//! use fipa_wasm_agents::platform::{AMS, AMSConfig};
//!
//! let ams = AMS::new(AMSConfig::default());
//! let ams_addr = ams.start();
//!
//! // Create an agent via AMS
//! ams_addr.send(AMSCreateAgent {
//!     name: "my-agent".to_string(),
//!     wasm_module: wasm_bytes,
//!     ..Default::default()
//! }).await?;
//! ```

pub mod ams;
pub mod df;

pub use ams::{AMS, AMSConfig, AMSCreateAgent, AMSDestroyAgent, AMSQueryAgents, AMSSuspendAgent, AMSResumeAgent};
pub use df::{DF, DFConfig, DFRegister, DFDeregister, DFSearch, DFSubscribe};
