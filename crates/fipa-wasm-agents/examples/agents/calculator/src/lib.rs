//! Calculator Agent
//!
//! A FIPA agent demonstrating:
//! - Service registration (registers as "calculator" service)
//! - Query protocol (QUERY-REF / INFORM-REF)
//! - Expression parsing
//!
//! Send QUERY-REF with math expression, receive INFORM-REF with result.
//!
//! Supported operations: +, -, *, /, %, ^
//! Examples: "2 + 3", "10 * 5", "2 ^ 8", "(3 + 4) * 2"

wit_bindgen::generate!({
    world: "agent",
    path: "wit/fipa.wit",
});

use fipa::agent::messaging::{self, AclMessage, Performative, ProtocolType};
use fipa::agent::lifecycle;
use fipa::agent::logging::{self, LogLevel};
use fipa::agent::services::{self, ServiceDescription};

/// Calculator Agent
struct CalculatorAgent;

/// Statistics
static mut QUERIES_HANDLED: u64 = 0;

impl Guest for CalculatorAgent {
    fn init() {
        let agent_id = lifecycle::get_agent_id();

        // Register the calculator service
        let service = ServiceDescription {
            name: "calculator".to_string(),
            description: "Mathematical expression evaluator".to_string(),
            protocols: vec![ProtocolType::Query],
            ontology: Some("math".to_string()),
            properties: vec![
                ("operations".to_string(), "+,-,*,/,%,^".to_string()),
                ("supports_parentheses".to_string(), "true".to_string()),
            ],
        };

        match services::register_service(&service) {
            Ok(_) => {
                logging::log(LogLevel::Info, "Registered 'calculator' service");
            }
            Err(e) => {
                logging::log(LogLevel::Error, &format!("Failed to register service: {:?}", e));
            }
        }

        logging::log(
            LogLevel::Info,
            &format!("Calculator Agent '{}' ready", agent_id.name),
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
        // Deregister service
        let _ = services::deregister_service("calculator");

        unsafe {
            logging::log(
                LogLevel::Info,
                &format!("Calculator Agent shutdown. {} queries handled", QUERIES_HANDLED),
            );
        }
    }

    fn execute_behavior(_behavior_id: u64, _behavior_name: String) -> bool {
        // Calculator agent doesn't use behaviors
        true
    }

    fn on_behavior_start(_behavior_id: u64, _behavior_name: String) {}

    fn on_behavior_end(_behavior_id: u64, _behavior_name: String) {}
}

fn handle_message(message: AclMessage) -> bool {
    match message.performative {
        Performative::QueryRef => {
            handle_query(&message);
            true
        }
        Performative::Request => {
            // Also accept requests (more lenient)
            handle_query(&message);
            true
        }
        _ => false,
    }
}

fn handle_query(message: &AclMessage) {
    let expression = String::from_utf8_lossy(&message.content);
    let expression = expression.trim();

    logging::log(
        LogLevel::Debug,
        &format!("Evaluating: '{}' for '{}'", expression, message.sender.name),
    );

    let (result, error) = evaluate_expression(expression);

    let (performative, content) = if let Some(err) = error {
        (Performative::Failure, format!("Error: {}", err))
    } else {
        unsafe { QUERIES_HANDLED += 1; }
        (Performative::InformRef, format!("{}", result))
    };

    let reply = AclMessage {
        message_id: format!("calc-reply-{}", message.message_id),
        performative,
        sender: lifecycle::get_agent_id(),
        receivers: vec![message.sender.clone()],
        protocol: Some(ProtocolType::Query),
        conversation_id: message.conversation_id.clone(),
        in_reply_to: Some(message.message_id.clone()),
        reply_by: None,
        language: Some("text/plain".to_string()),
        ontology: Some("math".to_string()),
        content: content.into_bytes(),
    };

    if let Err(e) = messaging::send_message(&reply) {
        logging::log(LogLevel::Error, &format!("Failed to send reply: {:?}", e));
    }
}

/// Simple expression evaluator
fn evaluate_expression(expr: &str) -> (f64, Option<String>) {
    // Remove whitespace
    let expr: String = expr.chars().filter(|c| !c.is_whitespace()).collect();

    if expr.is_empty() {
        return (0.0, Some("Empty expression".to_string()));
    }

    match parse_expression(&expr, 0) {
        Ok((result, _)) => (result, None),
        Err(e) => (0.0, Some(e)),
    }
}

/// Recursive descent parser for expressions
fn parse_expression(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    parse_additive(expr, pos)
}

fn parse_additive(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    let (mut left, mut pos) = parse_multiplicative(expr, pos)?;

    while pos < expr.len() {
        let c = expr.chars().nth(pos).unwrap();
        if c == '+' {
            let (right, new_pos) = parse_multiplicative(expr, pos + 1)?;
            left += right;
            pos = new_pos;
        } else if c == '-' {
            let (right, new_pos) = parse_multiplicative(expr, pos + 1)?;
            left -= right;
            pos = new_pos;
        } else {
            break;
        }
    }

    Ok((left, pos))
}

fn parse_multiplicative(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    let (mut left, mut pos) = parse_power(expr, pos)?;

    while pos < expr.len() {
        let c = expr.chars().nth(pos).unwrap();
        if c == '*' {
            let (right, new_pos) = parse_power(expr, pos + 1)?;
            left *= right;
            pos = new_pos;
        } else if c == '/' {
            let (right, new_pos) = parse_power(expr, pos + 1)?;
            if right == 0.0 {
                return Err("Division by zero".to_string());
            }
            left /= right;
            pos = new_pos;
        } else if c == '%' {
            let (right, new_pos) = parse_power(expr, pos + 1)?;
            left %= right;
            pos = new_pos;
        } else {
            break;
        }
    }

    Ok((left, pos))
}

fn parse_power(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    let (base, pos) = parse_unary(expr, pos)?;

    if pos < expr.len() && expr.chars().nth(pos).unwrap() == '^' {
        let (exp, new_pos) = parse_power(expr, pos + 1)?;
        Ok((base.powf(exp), new_pos))
    } else {
        Ok((base, pos))
    }
}

fn parse_unary(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    if pos >= expr.len() {
        return Err("Unexpected end of expression".to_string());
    }

    let c = expr.chars().nth(pos).unwrap();
    if c == '-' {
        let (val, new_pos) = parse_primary(expr, pos + 1)?;
        Ok((-val, new_pos))
    } else if c == '+' {
        parse_primary(expr, pos + 1)
    } else {
        parse_primary(expr, pos)
    }
}

fn parse_primary(expr: &str, pos: usize) -> Result<(f64, usize), String> {
    if pos >= expr.len() {
        return Err("Unexpected end of expression".to_string());
    }

    let c = expr.chars().nth(pos).unwrap();

    // Parentheses
    if c == '(' {
        let (val, new_pos) = parse_expression(expr, pos + 1)?;
        if new_pos >= expr.len() || expr.chars().nth(new_pos).unwrap() != ')' {
            return Err("Missing closing parenthesis".to_string());
        }
        return Ok((val, new_pos + 1));
    }

    // Number
    let mut end = pos;
    let chars: Vec<char> = expr.chars().collect();

    while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
        end += 1;
    }

    if end == pos {
        return Err(format!("Expected number at position {}", pos));
    }

    let num_str: String = chars[pos..end].iter().collect();
    match num_str.parse::<f64>() {
        Ok(n) => Ok((n, end)),
        Err(_) => Err(format!("Invalid number: {}", num_str)),
    }
}

export!(CalculatorAgent);
