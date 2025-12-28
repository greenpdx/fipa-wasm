//! Counter Agent
//!
//! A FIPA agent demonstrating persistent storage.
//! - Maintains a counter that persists across restarts/migrations
//! - Responds to increment/decrement/get commands
//!
//! Commands (sent as REQUEST with content):
//! - "increment" or "inc" - Add 1 to counter
//! - "decrement" or "dec" - Subtract 1 from counter
//! - "get" or "value" - Get current value
//! - "reset" - Reset to 0
//! - "add:N" - Add N to counter (e.g., "add:5")
//!
//! Response is INFORM with counter value

wit_bindgen::generate!({
    world: "agent",
    path: "wit/fipa.wit",
});

use fipa::agent::messaging::{self, AclMessage, Performative, ProtocolType};
use fipa::agent::lifecycle;
use fipa::agent::logging::{self, LogLevel};
use fipa::agent::storage;

const COUNTER_KEY: &str = "counter_value";

/// Counter Agent
struct CounterAgent;

impl Guest for CounterAgent {
    fn init() {
        let agent_id = lifecycle::get_agent_id();
        let value = load_counter();

        logging::log(
            LogLevel::Info,
            &format!(
                "Counter Agent '{}' initialized with value: {}",
                agent_id.name, value
            ),
        );
    }

    fn run() -> bool {
        if lifecycle::is_shutdown_requested() {
            return false;
        }

        while let Some(msg) = messaging::receive_message() {
            handle_message(msg);
        }

        true
    }

    fn shutdown() {
        let value = load_counter();
        logging::log(
            LogLevel::Info,
            &format!("Counter Agent shutting down. Final value: {}", value),
        );
    }

    fn execute_behavior(_behavior_id: u64, _behavior_name: String) -> bool {
        // Counter agent doesn't use behaviors
        true
    }

    fn on_behavior_start(_behavior_id: u64, _behavior_name: String) {}

    fn on_behavior_end(_behavior_id: u64, _behavior_name: String) {}
}

fn handle_message(message: AclMessage) -> bool {
        if message.performative != Performative::Request {
            return false;
        }

        let content = String::from_utf8_lossy(&message.content).to_lowercase();
        let content = content.trim();

        logging::log(
            LogLevel::Debug,
            &format!("Command from '{}': {}", message.sender.name, content),
        );

        let (new_value, response_text) = match content {
            "increment" | "inc" | "++" => {
                let v = load_counter() + 1;
                save_counter(v);
                (v, format!("Counter incremented to {}", v))
            }
            "decrement" | "dec" | "--" => {
                let v = load_counter().saturating_sub(1);
                save_counter(v);
                (v, format!("Counter decremented to {}", v))
            }
            "get" | "value" | "?" => {
                let v = load_counter();
                (v, format!("Counter value is {}", v))
            }
            "reset" | "clear" => {
                save_counter(0);
                (0, "Counter reset to 0".to_string())
            }
            cmd if cmd.starts_with("add:") => {
                if let Ok(n) = cmd[4..].parse::<i64>() {
                    let current = load_counter() as i64;
                    let new_val = (current + n).max(0) as u64;
                    save_counter(new_val);
                    (new_val, format!("Added {}, counter is now {}", n, new_val))
                } else {
                    let v = load_counter();
                    (v, "Invalid number format".to_string())
                }
            }
            cmd if cmd.starts_with("set:") => {
                if let Ok(n) = cmd[4..].parse::<u64>() {
                    save_counter(n);
                    (n, format!("Counter set to {}", n))
                } else {
                    let v = load_counter();
                    (v, "Invalid number format".to_string())
                }
            }
            _ => {
                let v = load_counter();
                (v, format!("Unknown command. Value is {}. Try: inc, dec, get, reset, add:N, set:N", v))
            }
        };

        // Send response
        let reply = AclMessage {
            message_id: format!("counter-reply-{}", message.message_id),
            performative: Performative::Inform,
            sender: lifecycle::get_agent_id(),
            receivers: vec![message.sender.clone()],
            protocol: Some(ProtocolType::Request),
            conversation_id: message.conversation_id,
            in_reply_to: Some(message.message_id),
            reply_by: None,
            language: Some("text/plain".to_string()),
            ontology: None,
            content: format!("{}:{}", new_value, response_text).into_bytes(),
        };

        if let Err(e) = messaging::send_message(&reply) {
            logging::log(LogLevel::Error, &format!("Failed to send reply: {:?}", e));
        }

        true
}

/// Load counter from persistent storage
fn load_counter() -> u64 {
    match storage::load(COUNTER_KEY) {
        Ok(data) if data.len() == 8 => {
            u64::from_le_bytes(data.try_into().unwrap())
        }
        _ => 0,
    }
}

/// Save counter to persistent storage
fn save_counter(value: u64) {
    if let Err(e) = storage::store(COUNTER_KEY, &value.to_le_bytes()) {
        logging::log(LogLevel::Error, &format!("Failed to save counter: {:?}", e));
    }
}

export!(CounterAgent);
