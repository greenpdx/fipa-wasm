//! Behavior Demo Agent
//!
//! Demonstrates JADE-style behaviors in a FIPA WASM agent.
//!
//! # Behaviors Demonstrated
//!
//! 1. **HeartbeatBehavior** (Ticker) - Logs a heartbeat every 5 seconds
//! 2. **MessageHandler** (Cyclic) - Processes incoming messages continuously
//! 3. **StartupTask** (OneShot) - Runs once at startup
//! 4. **DelayedTask** (Waker) - Runs once after 10 second delay
//! 5. **StateMachine** (FSM) - Demonstrates state machine transitions
//!
//! # Usage
//!
//! Deploy this agent to see behavior scheduling in action.
//! Send messages to trigger state machine transitions.

wit_bindgen::generate!({
    world: "behavior-agent",
    path: "wit/fipa.wit",
});

use fipa::agent::behaviors::{self, BehaviorConfig, BehaviorType, FsmTransition};
use fipa::agent::logging::{self, LogLevel};
use fipa::agent::lifecycle;
use fipa::agent::messaging::{self, AclMessage, Performative};

/// The Behavior Demo agent
struct BehaviorDemoAgent;

// Behavior IDs
static mut HEARTBEAT_ID: u64 = 0;
static mut MESSAGE_HANDLER_ID: u64 = 0;
static mut STARTUP_TASK_ID: u64 = 0;
static mut DELAYED_TASK_ID: u64 = 0;
static mut FSM_ID: u64 = 0;

// Statistics
static mut HEARTBEAT_COUNT: u64 = 0;
static mut MESSAGES_HANDLED: u64 = 0;

impl Guest for BehaviorDemoAgent {
    /// Initialize the agent and register behaviors
    fn init() {
        let agent_id = lifecycle::get_agent_id();
        logging::log(
            LogLevel::Info,
            &format!("Behavior Demo Agent '{}' initializing...", agent_id.name),
        );

        // 1. Register Heartbeat behavior (Ticker - every 5 seconds)
        let heartbeat_config = BehaviorConfig {
            behavior_type: BehaviorType::Ticker,
            tick_interval_ms: Some(5000),
            wake_after_ms: None,
            sub_behaviors: vec![],
            parallel_completion: None,
            parallel_n: None,
            fsm_initial_state: None,
            fsm_transitions: vec![],
        };

        match behaviors::add_behavior("heartbeat", &heartbeat_config) {
            Ok(id) => {
                unsafe { HEARTBEAT_ID = id; }
                logging::log(LogLevel::Info, &format!("Registered heartbeat behavior (id={})", id));
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register heartbeat: {:?}", e));
            }
        }

        // 2. Register Message Handler behavior (Cyclic)
        let handler_config = BehaviorConfig {
            behavior_type: BehaviorType::Cyclic,
            tick_interval_ms: None,
            wake_after_ms: None,
            sub_behaviors: vec![],
            parallel_completion: None,
            parallel_n: None,
            fsm_initial_state: None,
            fsm_transitions: vec![],
        };

        match behaviors::add_behavior("message-handler", &handler_config) {
            Ok(id) => {
                unsafe { MESSAGE_HANDLER_ID = id; }
                logging::log(LogLevel::Info, &format!("Registered message handler (id={})", id));
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register handler: {:?}", e));
            }
        }

        // 3. Register Startup Task (OneShot)
        let startup_config = BehaviorConfig {
            behavior_type: BehaviorType::OneShot,
            tick_interval_ms: None,
            wake_after_ms: None,
            sub_behaviors: vec![],
            parallel_completion: None,
            parallel_n: None,
            fsm_initial_state: None,
            fsm_transitions: vec![],
        };

        match behaviors::add_behavior("startup-task", &startup_config) {
            Ok(id) => {
                unsafe { STARTUP_TASK_ID = id; }
                logging::log(LogLevel::Info, &format!("Registered startup task (id={})", id));
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register startup: {:?}", e));
            }
        }

        // 4. Register Delayed Task (Waker - after 10 seconds)
        let delayed_config = BehaviorConfig {
            behavior_type: BehaviorType::Waker,
            tick_interval_ms: None,
            wake_after_ms: Some(10000), // 10 seconds
            sub_behaviors: vec![],
            parallel_completion: None,
            parallel_n: None,
            fsm_initial_state: None,
            fsm_transitions: vec![],
        };

        match behaviors::add_behavior("delayed-task", &delayed_config) {
            Ok(id) => {
                unsafe { DELAYED_TASK_ID = id; }
                logging::log(LogLevel::Info, &format!("Registered delayed task (id={})", id));
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register delayed: {:?}", e));
            }
        }

        // 5. Register FSM behavior
        let fsm_config = BehaviorConfig {
            behavior_type: BehaviorType::Fsm,
            tick_interval_ms: None,
            wake_after_ms: None,
            sub_behaviors: vec![],
            parallel_completion: None,
            parallel_n: None,
            fsm_initial_state: Some("idle".to_string()),
            fsm_transitions: vec![
                FsmTransition {
                    from_state: "idle".to_string(),
                    to_state: "processing".to_string(),
                    event: "start".to_string(),
                },
                FsmTransition {
                    from_state: "processing".to_string(),
                    to_state: "completed".to_string(),
                    event: "done".to_string(),
                },
                FsmTransition {
                    from_state: "processing".to_string(),
                    to_state: "idle".to_string(),
                    event: "cancel".to_string(),
                },
                FsmTransition {
                    from_state: "completed".to_string(),
                    to_state: "idle".to_string(),
                    event: "reset".to_string(),
                },
            ],
        };

        match behaviors::add_behavior("state-machine", &fsm_config) {
            Ok(id) => {
                unsafe { FSM_ID = id; }
                logging::log(LogLevel::Info, &format!("Registered FSM behavior (id={})", id));
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register FSM: {:?}", e));
            }
        }

        logging::log(LogLevel::Info, "All behaviors registered. Agent ready!");
    }

    /// Shutdown the agent
    fn shutdown() {
        unsafe {
            logging::log(
                LogLevel::Info,
                &format!(
                    "Behavior Demo Agent shutting down. Stats: {} heartbeats, {} messages handled",
                    HEARTBEAT_COUNT, MESSAGES_HANDLED
                ),
            );
        }
    }

    /// Execute a behavior's action
    fn execute_behavior(behavior_id: u64, behavior_name: String) -> bool {
        match behavior_name.as_str() {
            "heartbeat" => {
                unsafe {
                    HEARTBEAT_COUNT += 1;
                    logging::log(
                        LogLevel::Info,
                        &format!("💓 Heartbeat #{}", HEARTBEAT_COUNT),
                    );
                }
                false // Keep ticking
            }

            "message-handler" => {
                // Process all available messages
                while let Some(msg) = messaging::receive_message() {
                    handle_message(&msg);
                }
                false // Keep running (cyclic)
            }

            "startup-task" => {
                logging::log(LogLevel::Info, "🚀 Startup task executing (one-shot)");
                logging::log(LogLevel::Info, "   Performing initialization...");
                logging::log(LogLevel::Info, "   Startup task complete!");
                true // Done (one-shot)
            }

            "delayed-task" => {
                logging::log(LogLevel::Info, "⏰ Delayed task woke up after 10 seconds!");
                logging::log(LogLevel::Info, "   Performing delayed operation...");
                true // Done (waker)
            }

            "state-machine" => {
                // FSM behavior - just log current state
                unsafe {
                    if let Ok(state) = behaviors::fsm_current_state(FSM_ID) {
                        logging::log(LogLevel::Debug, &format!("FSM current state: {}", state));
                    }
                }
                false // Keep running
            }

            _ => {
                logging::log(LogLevel::Warn, &format!("Unknown behavior: {}", behavior_name));
                true // Unknown behavior, mark done
            }
        }
    }

    /// Called before a behavior starts (like JADE onStart)
    fn on_behavior_start(behavior_id: u64, behavior_name: String) {
        logging::log(
            LogLevel::Debug,
            &format!("Behavior '{}' (id={}) starting", behavior_name, behavior_id),
        );
    }

    /// Called after a behavior completes (like JADE onEnd)
    fn on_behavior_end(behavior_id: u64, behavior_name: String) {
        logging::log(
            LogLevel::Info,
            &format!("Behavior '{}' (id={}) completed", behavior_name, behavior_id),
        );
    }
}

/// Handle an incoming message
fn handle_message(msg: &AclMessage) {
    unsafe { MESSAGES_HANDLED += 1; }

    let content = String::from_utf8_lossy(&msg.content);
    logging::log(
        LogLevel::Info,
        &format!("📬 Received {:?} from '{}': {}", msg.performative, msg.sender.name, content),
    );

    // Handle FSM transitions via messages
    let content_lower = content.to_lowercase();
    if content_lower.contains("start") || content_lower.contains("done")
       || content_lower.contains("cancel") || content_lower.contains("reset") {
        let event = if content_lower.contains("start") { "start" }
                   else if content_lower.contains("done") { "done" }
                   else if content_lower.contains("cancel") { "cancel" }
                   else { "reset" };

        unsafe {
            match behaviors::fsm_event(FSM_ID, event) {
                Ok(new_state) => {
                    logging::log(
                        LogLevel::Info,
                        &format!("🔄 FSM transitioned to '{}' on event '{}'", new_state, event),
                    );
                }
                Err(e) => {
                    logging::log(
                        LogLevel::Warn,
                        &format!("FSM transition failed: {:?}", e),
                    );
                }
            }
        }
    }

    // Send acknowledgment
    let reply = AclMessage {
        message_id: format!("ack-{}", msg.message_id),
        performative: Performative::Inform,
        sender: fipa::agent::lifecycle::get_agent_id(),
        receivers: vec![msg.sender.clone()],
        protocol: msg.protocol,
        conversation_id: msg.conversation_id.clone(),
        in_reply_to: Some(msg.message_id.clone()),
        reply_by: None,
        language: Some("text/plain".to_string()),
        ontology: None,
        content: format!("Acknowledged: {}", content).into_bytes(),
    };

    if let Err(e) = messaging::send_message(&reply) {
        logging::log(LogLevel::Error, &format!("Failed to send ack: {:?}", e));
    }
}

export!(BehaviorDemoAgent);
