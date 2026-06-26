// behavior/mod.rs - JADE-style Behavior Scheduler
//
//! Behavior scheduling system following JADE patterns.
//!
//! Behaviors are registered by WASM agents and scheduled by the host runtime.
//! The host calls into the WASM agent to execute behavior actions.
//!
//! # Behavior Types
//!
//! - `OneShotBehaviour` - Executes once then completes
//! - `CyclicBehaviour` - Repeats indefinitely
//! - `TickerBehaviour` - Executes at fixed intervals
//! - `WakerBehaviour` - Executes once after delay
//! - `SequentialBehaviour` - Runs sub-behaviors in sequence
//! - `ParallelBehaviour` - Runs sub-behaviors concurrently
//! - `FSMBehaviour` - Finite state machine
//!
//! # Example
//!
//! ```ignore
//! // In WASM agent init():
//! let ticker_config = BehaviorConfig {
//!     behavior_type: BehaviorType::Ticker,
//!     tick_interval_ms: Some(1000),
//!     ..Default::default()
//! };
//! let id = behaviors::add_behavior("heartbeat", ticker_config)?;
//!
//! // In WASM agent execute_behavior():
//! fn execute_behavior(id: u64, name: &str) -> bool {
//!     match name {
//!         "heartbeat" => {
//!             logging::log(LogLevel::Info, "tick!");
//!             false // not done, keep ticking
//!         }
//!         _ => true // unknown behavior, mark done
//!     }
//! }
//! ```

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Behavior type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BehaviorType {
    /// Executes once then completes
    OneShot,
    /// Repeats indefinitely until blocked
    Cyclic,
    /// Executes at fixed time intervals
    Ticker,
    /// Executes once after a delay
    Waker,
    /// Runs sub-behaviors in sequence
    Sequential,
    /// Runs sub-behaviors concurrently
    Parallel,
    /// Finite state machine with named states
    FSM,
}

/// Parallel behavior completion condition
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParallelCompletion {
    /// Complete when all sub-behaviors finish
    WhenAll,
    /// Complete when any sub-behavior finishes
    WhenAny,
    /// Complete when N sub-behaviors finish
    WhenN(u32),
}

impl Default for ParallelCompletion {
    fn default() -> Self {
        Self::WhenAll
    }
}

/// FSM state transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FSMTransition {
    pub from_state: String,
    pub to_state: String,
    pub event: String,
}

/// Behavior configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BehaviorConfig {
    /// Behavior type
    pub behavior_type: Option<BehaviorType>,
    /// Tick interval for ticker behavior (ms)
    pub tick_interval_ms: Option<u64>,
    /// Wake delay for waker behavior (ms)
    pub wake_after_ms: Option<u64>,
    /// Sub-behavior IDs for composite behaviors
    pub sub_behaviors: Vec<u64>,
    /// Completion condition for parallel behaviors
    pub parallel_completion: Option<ParallelCompletion>,
    /// Initial state for FSM behaviors
    pub fsm_initial_state: Option<String>,
    /// Transitions for FSM behaviors
    pub fsm_transitions: Vec<FSMTransition>,
}

/// Behavior execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BehaviorStatus {
    /// Behavior is ready to run
    Ready,
    /// Behavior is currently executing
    Running,
    /// Behavior is blocked/paused
    Blocked,
    /// Behavior has completed
    Done,
}

/// Behavior error types
#[derive(Debug, Clone, thiserror::Error)]
pub enum BehaviorError {
    #[error("Behavior not found: {0}")]
    NotFound(u64),
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Behavior already running")]
    AlreadyRunning,
    #[error("Sub-behavior error: {0}")]
    SubBehaviorError(String),
    #[error("FSM error: {0}")]
    FSMError(String),
}

/// A registered behavior instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Behavior {
    /// Unique behavior ID
    pub id: u64,
    /// Behavior name
    pub name: String,
    /// Behavior type
    pub behavior_type: BehaviorType,
    /// Current status
    pub status: BehaviorStatus,
    /// Configuration
    pub config: BehaviorConfig,
    /// Last execution time (monotonic ms)
    pub last_run_ms: Option<u64>,
    /// Next scheduled run time (for ticker/waker)
    pub next_run_ms: Option<u64>,
    /// Execution count
    pub run_count: u64,
    /// Current FSM state
    pub fsm_current_state: Option<String>,
    /// Sequential behavior: current sub-behavior index
    pub sequential_index: usize,
    /// Parallel behavior: completed sub-behavior count
    pub parallel_completed: u32,
    /// Has onStart been called
    pub started: bool,
}

impl Behavior {
    /// Create a new behavior
    pub fn new(id: u64, name: String, behavior_type: BehaviorType, config: BehaviorConfig) -> Self {
        let fsm_current_state = config.fsm_initial_state.clone();

        Self {
            id,
            name,
            behavior_type,
            status: BehaviorStatus::Ready,
            config,
            last_run_ms: None,
            next_run_ms: None,
            run_count: 0,
            fsm_current_state,
            sequential_index: 0,
            parallel_completed: 0,
            started: false,
        }
    }

    /// Check if behavior should run now
    pub fn should_run(&self, current_ms: u64) -> bool {
        if self.status != BehaviorStatus::Ready {
            return false;
        }

        match self.behavior_type {
            BehaviorType::OneShot => self.run_count == 0,
            BehaviorType::Cyclic => true,
            BehaviorType::Ticker | BehaviorType::Waker => {
                self.next_run_ms.map(|t| current_ms >= t).unwrap_or(true)
            }
            BehaviorType::Sequential | BehaviorType::Parallel | BehaviorType::FSM => true,
        }
    }

    /// Mark behavior as done
    pub fn mark_done(&mut self) {
        self.status = BehaviorStatus::Done;
    }

    /// Update after execution
    pub fn after_run(&mut self, current_ms: u64, is_done: bool) {
        self.last_run_ms = Some(current_ms);
        self.run_count += 1;
        self.status = BehaviorStatus::Ready;

        if is_done {
            self.mark_done();
            return;
        }

        // Schedule next run for ticker
        if self.behavior_type == BehaviorType::Ticker {
            if let Some(interval) = self.config.tick_interval_ms {
                self.next_run_ms = Some(current_ms + interval);
            }
        }

        // Waker only runs once
        if self.behavior_type == BehaviorType::Waker {
            self.mark_done();
        }
    }
}

/// Behavior scheduler managing all behaviors for an agent
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BehaviorScheduler {
    /// Registered behaviors
    behaviors: HashMap<u64, Behavior>,
    /// Next behavior ID
    next_id: u64,
    /// Behaviors by name for lookup
    name_to_id: HashMap<String, u64>,
}

impl BehaviorScheduler {
    /// Create a new scheduler
    pub fn new() -> Self {
        Self {
            behaviors: HashMap::new(),
            next_id: 1,
            name_to_id: HashMap::new(),
        }
    }

    /// Add a new behavior
    pub fn add_behavior(
        &mut self,
        name: String,
        config: BehaviorConfig,
    ) -> Result<u64, BehaviorError> {
        let behavior_type = config.behavior_type.ok_or_else(|| {
            BehaviorError::InvalidConfig("behavior_type is required".to_string())
        })?;

        // Validate config based on type
        match behavior_type {
            BehaviorType::Ticker => {
                if config.tick_interval_ms.is_none() {
                    return Err(BehaviorError::InvalidConfig(
                        "tick_interval_ms required for Ticker".to_string(),
                    ));
                }
            }
            BehaviorType::Waker => {
                if config.wake_after_ms.is_none() {
                    return Err(BehaviorError::InvalidConfig(
                        "wake_after_ms required for Waker".to_string(),
                    ));
                }
            }
            BehaviorType::Sequential | BehaviorType::Parallel => {
                if config.sub_behaviors.is_empty() {
                    return Err(BehaviorError::InvalidConfig(
                        "sub_behaviors required for composite behavior".to_string(),
                    ));
                }
            }
            BehaviorType::FSM => {
                if config.fsm_initial_state.is_none() {
                    return Err(BehaviorError::InvalidConfig(
                        "fsm_initial_state required for FSM".to_string(),
                    ));
                }
            }
            _ => {}
        }

        let id = self.next_id;
        self.next_id += 1;

        let mut behavior = Behavior::new(id, name.clone(), behavior_type, config);

        // Set initial next_run_ms for waker
        if behavior_type == BehaviorType::Waker {
            if let Some(delay) = behavior.config.wake_after_ms {
                let now = chrono::Utc::now().timestamp_millis() as u64;
                behavior.next_run_ms = Some(now + delay);
            }
        }

        // Set initial next_run_ms for ticker (run immediately first time)
        if behavior_type == BehaviorType::Ticker {
            behavior.next_run_ms = Some(0); // Run immediately
        }

        self.behaviors.insert(id, behavior);
        self.name_to_id.insert(name, id);

        Ok(id)
    }

    /// Remove a behavior
    pub fn remove_behavior(&mut self, id: u64) -> Result<(), BehaviorError> {
        let behavior = self
            .behaviors
            .remove(&id)
            .ok_or(BehaviorError::NotFound(id))?;
        self.name_to_id.remove(&behavior.name);
        Ok(())
    }

    /// Block a behavior
    pub fn block_behavior(&mut self, id: u64) -> Result<(), BehaviorError> {
        let behavior = self
            .behaviors
            .get_mut(&id)
            .ok_or(BehaviorError::NotFound(id))?;

        if behavior.status == BehaviorStatus::Running {
            return Err(BehaviorError::AlreadyRunning);
        }

        behavior.status = BehaviorStatus::Blocked;
        Ok(())
    }

    /// Restart a blocked behavior
    pub fn restart_behavior(&mut self, id: u64) -> Result<(), BehaviorError> {
        let behavior = self
            .behaviors
            .get_mut(&id)
            .ok_or(BehaviorError::NotFound(id))?;

        if behavior.status == BehaviorStatus::Blocked {
            behavior.status = BehaviorStatus::Ready;
        }
        Ok(())
    }

    /// Get behavior status
    pub fn get_status(&self, id: u64) -> Result<BehaviorStatus, BehaviorError> {
        self.behaviors
            .get(&id)
            .map(|b| b.status)
            .ok_or(BehaviorError::NotFound(id))
    }

    /// Mark behavior as done
    pub fn behavior_done(&mut self, id: u64) {
        if let Some(b) = self.behaviors.get_mut(&id) {
            b.mark_done();
        }
    }

    /// Trigger FSM event
    pub fn fsm_event(&mut self, id: u64, event: &str) -> Result<String, BehaviorError> {
        let behavior = self
            .behaviors
            .get_mut(&id)
            .ok_or(BehaviorError::NotFound(id))?;

        if behavior.behavior_type != BehaviorType::FSM {
            return Err(BehaviorError::FSMError("Not an FSM behavior".to_string()));
        }

        let current_state = behavior
            .fsm_current_state
            .as_ref()
            .ok_or_else(|| BehaviorError::FSMError("No current state".to_string()))?;

        // Find matching transition
        for transition in &behavior.config.fsm_transitions {
            if transition.from_state == *current_state && transition.event == event {
                behavior.fsm_current_state = Some(transition.to_state.clone());
                return Ok(transition.to_state.clone());
            }
        }

        Err(BehaviorError::FSMError(format!(
            "No transition from '{}' on event '{}'",
            current_state, event
        )))
    }

    /// Get current FSM state
    pub fn fsm_current_state(&self, id: u64) -> Result<String, BehaviorError> {
        let behavior = self
            .behaviors
            .get(&id)
            .ok_or(BehaviorError::NotFound(id))?;

        behavior
            .fsm_current_state
            .clone()
            .ok_or_else(|| BehaviorError::FSMError("No current state".to_string()))
    }

    /// Reset behavior
    pub fn reset_behavior(&mut self, id: u64) -> Result<(), BehaviorError> {
        let behavior = self
            .behaviors
            .get_mut(&id)
            .ok_or(BehaviorError::NotFound(id))?;

        behavior.status = BehaviorStatus::Ready;
        behavior.run_count = 0;
        behavior.started = false;
        behavior.sequential_index = 0;
        behavior.parallel_completed = 0;
        behavior.fsm_current_state = behavior.config.fsm_initial_state.clone();

        Ok(())
    }

    /// List all behaviors
    pub fn list_behaviors(&self) -> Vec<(u64, String, BehaviorStatus)> {
        self.behaviors
            .values()
            .map(|b| (b.id, b.name.clone(), b.status))
            .collect()
    }

    /// Get behaviors that should run now
    pub fn get_runnable(&self, current_ms: u64) -> Vec<&Behavior> {
        self.behaviors
            .values()
            .filter(|b| b.should_run(current_ms))
            .collect()
    }

    /// Get behavior by ID
    pub fn get(&self, id: u64) -> Option<&Behavior> {
        self.behaviors.get(&id)
    }

    /// Get mutable behavior by ID
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Behavior> {
        self.behaviors.get_mut(&id)
    }

    /// Mark behavior as running
    pub fn mark_running(&mut self, id: u64) {
        if let Some(b) = self.behaviors.get_mut(&id) {
            b.status = BehaviorStatus::Running;
        }
    }

    /// Handle behavior completion
    pub fn handle_completion(&mut self, id: u64, current_ms: u64, is_done: bool) {
        if let Some(b) = self.behaviors.get_mut(&id) {
            b.after_run(current_ms, is_done);
        }
    }

    /// Check if behavior needs onStart callback
    pub fn needs_start(&self, id: u64) -> bool {
        self.behaviors
            .get(&id)
            .map(|b| !b.started)
            .unwrap_or(false)
    }

    /// Mark behavior as started
    pub fn mark_started(&mut self, id: u64) {
        if let Some(b) = self.behaviors.get_mut(&id) {
            b.started = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one_shot_behavior() {
        let mut scheduler = BehaviorScheduler::new();

        let config = BehaviorConfig {
            behavior_type: Some(BehaviorType::OneShot),
            ..Default::default()
        };

        let id = scheduler.add_behavior("test".to_string(), config).unwrap();

        // Should be runnable initially
        let runnable = scheduler.get_runnable(0);
        assert_eq!(runnable.len(), 1);

        // After running once, should be done
        scheduler.handle_completion(id, 0, true);
        let status = scheduler.get_status(id).unwrap();
        assert_eq!(status, BehaviorStatus::Done);
    }

    #[test]
    fn test_ticker_behavior() {
        let mut scheduler = BehaviorScheduler::new();

        let config = BehaviorConfig {
            behavior_type: Some(BehaviorType::Ticker),
            tick_interval_ms: Some(100),
            ..Default::default()
        };

        let id = scheduler.add_behavior("ticker".to_string(), config).unwrap();

        // Should run immediately
        let runnable = scheduler.get_runnable(0);
        assert_eq!(runnable.len(), 1);

        // After running, next run should be at 100ms
        scheduler.handle_completion(id, 0, false);
        let behavior = scheduler.get(id).unwrap();
        assert_eq!(behavior.next_run_ms, Some(100));

        // Should not be runnable at 50ms
        let runnable = scheduler.get_runnable(50);
        assert_eq!(runnable.len(), 0);

        // Should be runnable at 100ms
        let runnable = scheduler.get_runnable(100);
        assert_eq!(runnable.len(), 1);
    }

    #[test]
    fn test_fsm_behavior() {
        let mut scheduler = BehaviorScheduler::new();

        let config = BehaviorConfig {
            behavior_type: Some(BehaviorType::FSM),
            fsm_initial_state: Some("idle".to_string()),
            fsm_transitions: vec![
                FSMTransition {
                    from_state: "idle".to_string(),
                    to_state: "running".to_string(),
                    event: "start".to_string(),
                },
                FSMTransition {
                    from_state: "running".to_string(),
                    to_state: "idle".to_string(),
                    event: "stop".to_string(),
                },
            ],
            ..Default::default()
        };

        let id = scheduler.add_behavior("fsm".to_string(), config).unwrap();

        // Initial state
        assert_eq!(scheduler.fsm_current_state(id).unwrap(), "idle");

        // Transition
        let new_state = scheduler.fsm_event(id, "start").unwrap();
        assert_eq!(new_state, "running");

        // Invalid transition
        let result = scheduler.fsm_event(id, "start");
        assert!(result.is_err());
    }
}
