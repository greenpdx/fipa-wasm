//! Ping-Pong Agent
//!
//! A simple FIPA agent that demonstrates basic messaging.
//! - Receives REQUEST messages containing "ping"
//! - Responds with INFORM messages containing "pong"
//!
//! This example shows:
//! - Basic message handling
//! - Using the FIPA Request protocol
//! - Sending replies

wit_bindgen::generate!({
    world: "minimal-agent",
    path: "wit/fipa.wit",
});

use fipa::agent::messaging::{self, AclMessage, Performative, ProtocolType};
use fipa::agent::lifecycle;
use fipa::agent::logging::{self, LogLevel};

/// The Ping-Pong agent
struct PingPongAgent;

/// Message statistics
static mut PINGS_RECEIVED: u64 = 0;
static mut PONGS_SENT: u64 = 0;

impl Guest for PingPongAgent {
    /// Initialize the agent
    fn init() {
        let agent_id = lifecycle::get_agent_id();
        logging::log(
            LogLevel::Info,
            &format!("Ping-Pong Agent '{}' initialized", agent_id.name),
        );
        logging::log(LogLevel::Info, "Ready to receive ping messages!");
    }

    /// Main loop - process incoming messages
    fn run() -> bool {
        // Check for shutdown
        if lifecycle::is_shutdown_requested() {
            logging::log(LogLevel::Info, "Shutdown requested");
            return false;
        }

        // Process all available messages
        while let Some(msg) = messaging::receive_message() {
            handle_message_internal(&msg);
        }

        true // Keep running
    }

    /// Shutdown handler
    fn shutdown() {
        unsafe {
            logging::log(
                LogLevel::Info,
                &format!(
                    "Ping-Pong Agent shutting down. Stats: {} pings received, {} pongs sent",
                    PINGS_RECEIVED, PONGS_SENT
                ),
            );
        }
    }
}

/// Handle an incoming message
fn handle_message_internal(msg: &AclMessage) {
    logging::log(
        LogLevel::Debug,
        &format!(
            "Received {:?} from '{}': {} bytes",
            msg.performative,
            msg.sender.name,
            msg.content.len()
        ),
    );

    match msg.performative {
        Performative::Request => {
            // Check if content is "ping"
            let content = String::from_utf8_lossy(&msg.content);
            if content.to_lowercase().contains("ping") {
                unsafe { PINGS_RECEIVED += 1; }
                logging::log(LogLevel::Info, &format!("Received PING from '{}'", msg.sender.name));
                send_pong(msg);
            } else {
                logging::log(
                    LogLevel::Debug,
                    &format!("Ignoring request with content: {}", content),
                );
            }
        }
        Performative::Inform => {
            // Just log inform messages
            let content = String::from_utf8_lossy(&msg.content);
            logging::log(LogLevel::Debug, &format!("Received inform: {}", content));
        }
        _ => {
            logging::log(
                LogLevel::Debug,
                &format!("Unhandled performative: {:?}", msg.performative),
            );
        }
    }
}

/// Send a pong response
fn send_pong(original: &AclMessage) {
    let my_id = lifecycle::get_agent_id();

    let reply = AclMessage {
        message_id: format!("pong-{}", original.message_id),
        performative: Performative::Inform,
        sender: my_id,
        receivers: vec![original.sender.clone()],
        protocol: Some(ProtocolType::Request),
        conversation_id: original.conversation_id.clone(),
        in_reply_to: Some(original.message_id.clone()),
        reply_by: None,
        language: Some("text/plain".to_string()),
        ontology: None,
        content: b"pong".to_vec(),
    };

    match messaging::send_message(&reply) {
        Ok(msg_id) => {
            unsafe { PONGS_SENT += 1; }
            logging::log(
                LogLevel::Info,
                &format!("Sent PONG to '{}' (msg: {})", original.sender.name, msg_id),
            );
        }
        Err(e) => {
            logging::log(
                LogLevel::Error,
                &format!("Failed to send pong: {:?}", e),
            );
        }
    }
}

export!(PingPongAgent);
