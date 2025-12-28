// tools/mod.rs - FIPA Platform Tools
//
//! Platform tools and utilities for FIPA agents
//!
//! This module provides:
//! - Message sniffer for debugging and monitoring
//! - Web dashboard for platform visualization
//! - Additional monitoring and management tools

pub mod dashboard;
pub mod sniffer;

pub use dashboard::{Dashboard, DashboardConfig, DashboardState, SharedState};
pub use sniffer::{MessageSniffer, SnifferConfig, SnifferFilter, MessageTrace, TraceEntry};
