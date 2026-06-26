// examples/request_agent.rs
// Example WASM agent that handles request protocol

// This would be compiled to WASM with: cargo build --target wasm32-wasip1

#![no_main]

// Import the FIPA host functions (these would be generated from WIT)
mod fipa {
    pub fn send_message(
        receiver: &str,
        performative: u8,
        content: &str,
        protocol: u8,
    ) -> Result<u64, String> {
        // This is a placeholder - real implementation would use WIT bindings
        Ok(0)
    }

    pub fn receive_message() -> Result<Option<Message>, String> {
        Ok(None)
    }

    pub fn log(level: u8, message: &str) {
        // Placeholder
    }

    pub struct Message {
        pub sender: String,
        pub performative: u8,
        pub content: String,
        pub conversation_id: Option<String>,
    }
}

// Performative constants
const PERFORMATIVE_REQUEST: u8 = 20;
const PERFORMATIVE_AGREE: u8 = 1;
const PERFORMATIVE_INFORM_RESULT: u8 = 11;
const PERFORMATIVE_REFUSE: u8 = 18;

// Protocol constants
const PROTOCOL_REQUEST: u8 = 0;

static mut AGENT_STATE: Option<AgentState> = None;

struct AgentState {
    requests_handled: u32,
    active_conversations: Vec<String>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            requests_handled: 0,
            active_conversations: Vec::new(),
        }
    }
}

#[no_mangle]
pub extern "C" fn init() {
    fipa::log(1, "Request handler agent initializing...");
    unsafe {
        AGENT_STATE = Some(AgentState::default());
    }
    fipa::log(1, "Request handler agent initialized");
}

#[no_mangle]
pub extern "C" fn run() {
    fipa::log(1, "Agent started - waiting for requests");

    loop {
        // Check for incoming messages
        if let Ok(Some(msg)) = fipa::receive_message() {
            handle_message(msg);
        }

        // Do periodic work
        check_migration();

        // Sleep briefly
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[no_mangle]
pub extern "C" fn shutdown() {
    fipa::log(1, "Agent shutting down gracefully");
}

fn handle_message(msg: fipa::Message) {
    fipa::log(1, &format!("Received message from: {}", msg.sender));

    match msg.performative {
        PERFORMATIVE_REQUEST => {
            handle_request(msg);
        }
        _ => {
            fipa::log(2, &format!("Unknown performative: {}", msg.performative));
        }
    }
}

fn handle_request(msg: fipa::Message) {
    fipa::log(1, &format!("Processing request: {}", msg.content));

    // Decide if we can handle this request
    let can_handle = evaluate_request(&msg.content);

    if can_handle {
        // Send agree
        let _ = fipa::send_message(
            &msg.sender,
            PERFORMATIVE_AGREE,
            "I can handle this request",
            PROTOCOL_REQUEST,
        );

        // Process the request
        let result = process_request(&msg.content);

        // Send result
        let _ = fipa::send_message(
            &msg.sender,
            PERFORMATIVE_INFORM_RESULT,
            &result,
            PROTOCOL_REQUEST,
        );

        // Update state
        unsafe {
            if let Some(state) = &mut AGENT_STATE {
                state.requests_handled += 1;
                fipa::log(
                    1,
                    &format!("Total requests handled: {}", state.requests_handled),
                );
            }
        }
    } else {
        // Send refuse
        let _ = fipa::send_message(
            &msg.sender,
            PERFORMATIVE_REFUSE,
            "Cannot handle this request",
            PROTOCOL_REQUEST,
        );
    }
}

fn evaluate_request(content: &str) -> bool {
    // Simple evaluation - in real agent this would be more sophisticated
    !content.is_empty() && content.len() < 1000
}

fn process_request(content: &str) -> String {
    // Process the request and generate result
    fipa::log(1, "Processing request...");

    // Simulate some work
    std::thread::sleep(std::time::Duration::from_millis(50));

    format!("Processed: {}", content.to_uppercase())
}

fn check_migration() {
    // Check if we should migrate to another node
    // This is a placeholder - real implementation would check load, network conditions, etc.
}
