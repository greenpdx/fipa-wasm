//! fipa-agent-new - FIPA Agent Project Generator
//!
//! Creates new WASM agent projects from templates.
//!
//! Usage:
//!   fipa-agent-new <name> [--template <type>] [--output <dir>]
//!
//! Templates:
//!   full     - Full-featured agent with all interfaces
//!   minimal  - Simple agent with messaging/lifecycle/logging
//!   stateless - Pure message handler

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::fs;
use std::path::{Path, PathBuf};

/// FIPA Agent Project Generator
#[derive(Parser, Debug)]
#[command(name = "fipa-agent-new")]
#[command(author = "FIPA-WASM Team")]
#[command(version)]
#[command(about = "Create a new FIPA WASM agent project", long_about = None)]
struct Args {
    /// Name of the agent (will be used as package name)
    name: String,

    /// Template to use
    #[arg(short, long, value_enum, default_value = "full")]
    template: TemplateType,

    /// Output directory (default: ./<name>)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Overwrite existing directory
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TemplateType {
    /// Full-featured agent with all FIPA interfaces
    Full,
    /// Minimal agent with messaging, lifecycle, logging
    Minimal,
    /// Stateless message handler
    Stateless,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate name
    let agent_name = validate_name(&args.name)?;
    let agent_type = to_pascal_case(&agent_name);

    // Determine output directory
    let output_dir = args.output.unwrap_or_else(|| PathBuf::from(&agent_name));

    // Check if directory exists
    if output_dir.exists() && !args.force {
        anyhow::bail!(
            "Directory '{}' already exists. Use --force to overwrite.",
            output_dir.display()
        );
    }

    println!("Creating FIPA agent project '{}'...", agent_name);
    println!("  Template: {:?}", args.template);
    println!("  Output: {}", output_dir.display());

    // Create directory structure
    fs::create_dir_all(output_dir.join("src"))?;
    fs::create_dir_all(output_dir.join("wit"))?;

    // Generate files from templates
    generate_cargo_toml(&output_dir, &agent_name, args.template)?;
    generate_lib_rs(&output_dir, &agent_name, &agent_type, args.template)?;
    generate_readme(&output_dir, &agent_name, args.template)?;
    copy_wit_file(&output_dir)?;

    println!();
    println!("Agent project created successfully!");
    println!();
    println!("Next steps:");
    println!("  cd {}", output_dir.display());
    println!("  rustup target add wasm32-wasip2");
    println!("  cargo build --release --target wasm32-wasip2");
    println!();
    println!("The WASM module will be at:");
    println!("  target/wasm32-wasip2/release/{}.wasm", agent_name.replace('-', "_"));

    Ok(())
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.to_lowercase();

    if name.is_empty() {
        anyhow::bail!("Agent name cannot be empty");
    }

    if !name.chars().next().unwrap().is_ascii_alphabetic() {
        anyhow::bail!("Agent name must start with a letter");
    }

    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Agent name can only contain letters, numbers, hyphens, and underscores");
    }

    Ok(name)
}

fn to_pascal_case(s: &str) -> String {
    s.split(|c| c == '-' || c == '_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

fn generate_cargo_toml(dir: &Path, name: &str, template: TemplateType) -> Result<()> {
    let world = match template {
        TemplateType::Full => "agent",
        TemplateType::Minimal => "minimal-agent",
        TemplateType::Stateless => "stateless-handler",
    };

    let content = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
description = "FIPA WASM Agent - {name}"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"

# Optional: for content serialization
# serde = {{ version = "1.0", features = ["derive"] }}
# serde_json = "1.0"

[profile.release]
opt-level = "s"
lto = true
strip = true

# WIT configuration
# World: {world}
"#
    );

    fs::write(dir.join("Cargo.toml"), content)
        .context("Failed to write Cargo.toml")?;

    Ok(())
}

fn generate_lib_rs(dir: &Path, name: &str, agent_type: &str, template: TemplateType) -> Result<()> {
    let content = match template {
        TemplateType::Full => generate_full_template(name, agent_type),
        TemplateType::Minimal => generate_minimal_template(name, agent_type),
        TemplateType::Stateless => generate_stateless_template(name, agent_type),
    };

    fs::write(dir.join("src/lib.rs"), content)
        .context("Failed to write src/lib.rs")?;

    Ok(())
}

fn generate_full_template(name: &str, agent_type: &str) -> String {
    format!(
        r#"//! {name} - FIPA WASM Agent
//!
//! A full-featured FIPA agent with access to all host interfaces.

wit_bindgen::generate!({{
    world: "agent",
    path: "wit/fipa.wit",
}});

use exports::fipa::agent::guest::Guest;
use fipa::agent::messaging::{{self, AclMessage, Performative}};
use fipa::agent::lifecycle;
use fipa::agent::logging::{{self, LogLevel}};
use fipa::agent::storage;

struct {agent_type} {{
    initialized: bool,
}}

static mut AGENT: Option<{agent_type}> = None;

fn get_agent() -> &'static mut {agent_type} {{
    unsafe {{
        AGENT.get_or_insert_with(|| {agent_type} {{ initialized: false }})
    }}
}}

impl Guest for {agent_type} {{
    fn init() {{
        let agent = get_agent();
        let id = lifecycle::get_agent_id();
        logging::log(LogLevel::Info, &format!("{name} '{{}}' initialized", id.name));
        agent.initialized = true;
    }}

    fn run() -> bool {{
        if lifecycle::is_shutdown_requested() {{
            return false;
        }}

        while let Some(msg) = messaging::receive_message() {{
            Self::handle_message(msg);
        }}

        true
    }}

    fn shutdown() {{
        logging::log(LogLevel::Info, "{name} shutting down");
    }}

    fn handle_message(message: AclMessage) -> bool {{
        logging::log(LogLevel::Debug, &format!(
            "Received {{:?}} from '{{}}'",
            message.performative,
            message.sender.name
        ));

        match message.performative {{
            Performative::Request => {{
                // Handle request
                let reply = AclMessage {{
                    message_id: format!("reply-{{}}", message.message_id),
                    performative: Performative::Inform,
                    sender: lifecycle::get_agent_id(),
                    receivers: vec![message.sender.clone()],
                    protocol: message.protocol,
                    conversation_id: message.conversation_id,
                    in_reply_to: Some(message.message_id),
                    reply_by: None,
                    language: None,
                    ontology: None,
                    content: b"OK".to_vec(),
                }};
                let _ = messaging::send_message(&reply);
                true
            }}
            _ => false,
        }}
    }}
}}

export!({agent_type});
"#
    )
}

fn generate_minimal_template(name: &str, agent_type: &str) -> String {
    format!(
        r#"//! {name} - Minimal FIPA WASM Agent

wit_bindgen::generate!({{
    world: "minimal-agent",
    path: "wit/fipa.wit",
}});

use exports::fipa::agent::guest::Guest;
use fipa::agent::messaging::{{self, AclMessage, Performative}};
use fipa::agent::lifecycle;
use fipa::agent::logging::{{self, LogLevel}};

struct {agent_type};

impl Guest for {agent_type} {{
    fn init() {{
        let id = lifecycle::get_agent_id();
        logging::log(LogLevel::Info, &format!("{name} '{{}}' started", id.name));
    }}

    fn run() -> bool {{
        if lifecycle::is_shutdown_requested() {{
            return false;
        }}

        while let Some(msg) = messaging::receive_message() {{
            process_message(&msg);
        }}

        true
    }}

    fn shutdown() {{
        logging::log(LogLevel::Info, "{name} shutting down");
    }}
}}

fn process_message(msg: &AclMessage) {{
    logging::log(LogLevel::Debug, &format!("Message from: {{}}", msg.sender.name));
    // Add your message handling logic here
}}

export!({agent_type});
"#
    )
}

fn generate_stateless_template(name: &str, agent_type: &str) -> String {
    format!(
        r#"//! {name} - Stateless FIPA Handler

wit_bindgen::generate!({{
    world: "stateless-handler",
    path: "wit/fipa.wit",
}});

use exports::fipa::agent::guest::Guest;
use fipa::agent::messaging::{{AclMessage, AgentId, Performative}};
use fipa::agent::logging::{{self, LogLevel}};

struct {agent_type};

impl Guest for {agent_type} {{
    fn handle_message(message: AclMessage) -> Option<AclMessage> {{
        logging::log(LogLevel::Debug, &format!(
            "Handling {{:?}} from '{{}}'",
            message.performative,
            message.sender.name
        ));

        match message.performative {{
            Performative::Request => {{
                Some(AclMessage {{
                    message_id: format!("reply-{{}}", message.message_id),
                    performative: Performative::Inform,
                    sender: AgentId {{
                        name: "{name}".to_string(),
                        addresses: vec![],
                    }},
                    receivers: vec![message.sender],
                    protocol: message.protocol,
                    conversation_id: message.conversation_id,
                    in_reply_to: Some(message.message_id),
                    reply_by: None,
                    language: None,
                    ontology: None,
                    content: b"Processed".to_vec(),
                }})
            }}
            _ => None,
        }}
    }}
}}

export!({agent_type});
"#
    )
}

fn generate_readme(dir: &Path, name: &str, template: TemplateType) -> Result<()> {
    let template_name = match template {
        TemplateType::Full => "Full",
        TemplateType::Minimal => "Minimal",
        TemplateType::Stateless => "Stateless",
    };

    let content = format!(
        r#"# {name}

A FIPA-compliant WASM agent ({template_name} template).

## Building

```bash
# Install WASM target (first time only)
rustup target add wasm32-wasip2

# Build the agent
cargo build --release --target wasm32-wasip2
```

The compiled WASM module will be at:
`target/wasm32-wasip2/release/{underscored}.wasm`

## Deploying

```bash
# Deploy to local FIPA node
fipa-cli deploy ./target/wasm32-wasip2/release/{underscored}.wasm
```

## Testing

Use the fipa-cli to send test messages:

```bash
fipa-cli send --to {name} --performative request --content "test"
```

## License

MIT
"#,
        underscored = name.replace('-', "_")
    );

    fs::write(dir.join("README.md"), content)
        .context("Failed to write README.md")?;

    Ok(())
}

fn copy_wit_file(dir: &Path) -> Result<()> {
    // Try to find the WIT file in common locations
    let possible_paths = [
        PathBuf::from("fipa.wit"),
        PathBuf::from("../fipa.wit"),
        PathBuf::from("../../fipa.wit"),
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config/fipa/fipa.wit"))
            .unwrap_or_default(),
    ];

    for path in &possible_paths {
        if path.exists() {
            fs::copy(path, dir.join("wit/fipa.wit"))
                .context("Failed to copy fipa.wit")?;
            return Ok(());
        }
    }

    // If not found, create a placeholder with instructions
    let placeholder = r#"// fipa.wit - FIPA Agent Interface
//
// This file should contain the FIPA WIT definitions.
// Copy fipa.wit from the fipa-wasm-agents repository to this location.
//
// Download from: https://github.com/your-repo/fipa-wasm-agents/blob/main/fipa.wit
"#;

    fs::write(dir.join("wit/fipa.wit"), placeholder)
        .context("Failed to create wit placeholder")?;

    eprintln!("Warning: Could not find fipa.wit. A placeholder was created.");
    eprintln!("Please copy the actual fipa.wit file to {}/wit/", dir.display());

    Ok(())
}
