// examples/contractor_agent.rs
// Example WASM agent that participates in Contract Net protocol and can migrate

#![no_main]

mod fipa {
    // Host function imports (placeholders)
    pub fn send_message(
        receiver: &str,
        performative: u8,
        content: &str,
        protocol: u8,
    ) -> Result<u64, String> {
        Ok(0)
    }

    pub fn receive_message() -> Result<Option<Message>, String> {
        Ok(None)
    }

    pub fn log(level: u8, message: &str) {}

    pub fn get_current_node() -> String {
        "node-1".to_string()
    }

    pub fn list_nodes() -> Vec<NodeInfo> {
        vec![]
    }

    pub fn migrate_to(node_id: &str) -> Result<(), String> {
        Ok(())
    }

    pub struct Message {
        pub sender: String,
        pub performative: u8,
        pub content: String,
        pub conversation_id: Option<String>,
    }

    pub struct NodeInfo {
        pub id: String,
        pub load: f32,
        pub latency_ms: u32,
    }
}

// Performative constants
const PERFORMATIVE_CFP: u8 = 3;
const PERFORMATIVE_PROPOSE: u8 = 14;
const PERFORMATIVE_ACCEPT_PROPOSAL: u8 = 0;
const PERFORMATIVE_REJECT_PROPOSAL: u8 = 19;
const PERFORMATIVE_INFORM_RESULT: u8 = 11;
const PERFORMATIVE_REFUSE: u8 = 18;

// Protocol constants
const PROTOCOL_CONTRACT_NET: u8 = 3;

static mut AGENT_STATE: Option<ContractorState> = None;

struct ContractorState {
    skills: Vec<String>,
    current_capacity: f32,
    completed_tasks: u32,
    active_tasks: Vec<String>,
    migration_threshold: f32,
}

impl Default for ContractorState {
    fn default() -> Self {
        Self {
            skills: vec!["data-processing".to_string(), "analysis".to_string()],
            current_capacity: 1.0, // 1.0 = full capacity available
            completed_tasks: 0,
            active_tasks: Vec::new(),
            migration_threshold: 0.5, // Migrate if other nodes have >50% better conditions
        }
    }
}

#[no_mangle]
pub extern "C" fn init() {
    fipa::log(1, "Contractor agent initializing...");
    unsafe {
        AGENT_STATE = Some(ContractorState::default());
    }
    fipa::log(1, "Contractor agent ready to bid on tasks");
}

#[no_mangle]
pub extern "C" fn run() {
    loop {
        // Check for incoming messages
        if let Ok(Some(msg)) = fipa::receive_message() {
            handle_message(msg);
        }

        // Periodic tasks
        check_migration_opportunity();
        update_capacity();

        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[no_mangle]
pub extern "C" fn shutdown() {
    fipa::log(1, "Contractor agent shutting down");
}

fn handle_message(msg: fipa::Message) {
    match msg.performative {
        PERFORMATIVE_CFP => {
            handle_call_for_proposals(msg);
        }
        PERFORMATIVE_ACCEPT_PROPOSAL => {
            handle_proposal_accepted(msg);
        }
        PERFORMATIVE_REJECT_PROPOSAL => {
            handle_proposal_rejected(msg);
        }
        _ => {
            fipa::log(2, &format!("Unexpected performative: {}", msg.performative));
        }
    }
}

fn handle_call_for_proposals(msg: fipa::Message) {
    fipa::log(1, &format!("Received CFP: {}", msg.content));

    unsafe {
        if let Some(state) = &AGENT_STATE {
            // Parse task requirements
            let task_info = parse_task(&msg.content);

            // Check if we have the skills and capacity
            if can_handle_task(&task_info, state) {
                // Calculate our bid
                let bid = calculate_bid(&task_info, state);

                fipa::log(
                    1,
                    &format!("Submitting proposal with bid: ${:.2}", bid),
                );

                // Send proposal
                let proposal_content = format!(
                    "{{\"price\": {}, \"completion_time\": 60, \"quality\": 0.95}}",
                    bid
                );

                let _ = fipa::send_message(
                    &msg.sender,
                    PERFORMATIVE_PROPOSE,
                    &proposal_content,
                    PROTOCOL_CONTRACT_NET,
                );
            } else {
                // Refuse - don't have capacity or skills
                fipa::log(1, "Cannot handle this task - refusing");
                let _ = fipa::send_message(
                    &msg.sender,
                    PERFORMATIVE_REFUSE,
                    "Insufficient capacity or skills",
                    PROTOCOL_CONTRACT_NET,
                );
            }
        }
    }
}

fn handle_proposal_accepted(msg: fipa::Message) {
    fipa::log(1, "Our proposal was accepted!");

    // Execute the task
    let result = execute_task(&msg.content);

    // Send results
    let _ = fipa::send_message(
        &msg.sender,
        PERFORMATIVE_INFORM_RESULT,
        &result,
        PROTOCOL_CONTRACT_NET,
    );

    unsafe {
        if let Some(state) = &mut AGENT_STATE {
            state.completed_tasks += 1;
            fipa::log(1, &format!("Completed tasks: {}", state.completed_tasks));
        }
    }
}

fn handle_proposal_rejected(_msg: fipa::Message) {
    fipa::log(1, "Our proposal was rejected");
}

fn parse_task(content: &str) -> TaskInfo {
    // Parse JSON task description
    TaskInfo {
        task_type: "data-processing".to_string(),
        complexity: 5,
        deadline_minutes: 60,
    }
}

struct TaskInfo {
    task_type: String,
    complexity: u32,
    deadline_minutes: u32,
}

fn can_handle_task(task: &TaskInfo, state: &ContractorState) -> bool {
    // Check if we have the required skill
    if !state.skills.contains(&task.task_type) {
        return false;
    }

    // Check if we have capacity
    let required_capacity = task.complexity as f32 * 0.1;
    state.current_capacity >= required_capacity
}

fn calculate_bid(task: &TaskInfo, state: &ContractorState) -> f64 {
    // Calculate competitive bid based on complexity and our capacity
    let base_price = task.complexity as f64 * 10.0;

    // Discount if we have excess capacity
    let capacity_multiplier = if state.current_capacity > 0.7 {
        0.8 // 20% discount when we have lots of capacity
    } else {
        1.0
    };

    base_price * capacity_multiplier
}

fn execute_task(content: &str) -> String {
    fipa::log(1, "Executing task...");

    // Simulate task execution
    std::thread::sleep(std::time::Duration::from_secs(1));

    format!("Task completed successfully: {}", content)
}

fn update_capacity() {
    unsafe {
        if let Some(state) = &mut AGENT_STATE {
            // Update based on active tasks
            let used_capacity = state.active_tasks.len() as f32 * 0.2;
            state.current_capacity = (1.0 - used_capacity).max(0.0);
        }
    }
}

fn check_migration_opportunity() {
    // Get current node
    let current_node = fipa::get_current_node();

    // Get available nodes
    let nodes = fipa::list_nodes();

    unsafe {
        if let Some(state) = &AGENT_STATE {
            // Find best node based on load and latency
            let best_node = find_best_node(&nodes, &current_node, state);

            if let Some(target_node) = best_node {
                fipa::log(
                    1,
                    &format!("Migrating to better node: {}", target_node),
                );

                match fipa::migrate_to(&target_node) {
                    Ok(_) => {
                        fipa::log(1, "Migration initiated");
                    }
                    Err(e) => {
                        fipa::log(3, &format!("Migration failed: {}", e));
                    }
                }
            }
        }
    }
}

fn find_best_node(
    nodes: &[fipa::NodeInfo],
    current: &str,
    state: &ContractorState,
) -> Option<String> {
    let mut best_node: Option<String> = None;
    let mut best_score = 0.0;

    for node in nodes {
        if node.id == current {
            continue;
        }

        // Calculate score based on load and latency
        let load_score = 1.0 - node.load;
        let latency_score = 1.0 - (node.latency_ms as f32 / 1000.0).min(1.0);
        let score = load_score * 0.7 + latency_score * 0.3;

        if score > best_score && score > state.migration_threshold {
            best_score = score;
            best_node = Some(node.id.clone());
        }
    }

    best_node
}
