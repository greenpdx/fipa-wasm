// bin/tui.rs - FIPA Terminal User Interface
//
//! FIPA Terminal User Interface
//!
//! An interactive terminal UI for managing FIPA agent platforms.
//!
//! # Features
//!
//! - Agent browser with vim-style navigation
//! - Real-time message stream view
//! - Service discovery panel
//! - Metrics dashboard
//! - Keyboard shortcuts for all actions

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};
use std::io;
use std::time::{Duration, Instant};
use tonic::transport::Channel;

// Import generated proto types
pub mod fipa {
    pub mod v1 {
        tonic::include_proto!("fipa.v1");
    }
}

use fipa::v1::{
    fipa_agent_service_client::FipaAgentServiceClient, FindServiceRequest, HealthCheckRequest,
    NodeInfoRequest,
};

/// FIPA TUI - Terminal User Interface
#[derive(Parser, Debug)]
#[command(name = "fipa-tui")]
#[command(author = "FIPA WASM Agents")]
#[command(version)]
#[command(about = "FIPA Agent System TUI - Interactive terminal interface")]
struct Args {
    /// Node address to connect to (host:port)
    #[arg(short, long, default_value = "http://localhost:9000")]
    node: String,

    /// Refresh interval in milliseconds
    #[arg(short, long, default_value = "1000")]
    refresh: u64,
}

/// Application tabs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Agents,
    Services,
    Messages,
    Metrics,
    Help,
}

impl Tab {
    fn titles() -> Vec<&'static str> {
        vec!["Agents", "Services", "Messages", "Metrics", "Help"]
    }

    fn index(&self) -> usize {
        match self {
            Tab::Agents => 0,
            Tab::Services => 1,
            Tab::Messages => 2,
            Tab::Metrics => 3,
            Tab::Help => 4,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            0 => Tab::Agents,
            1 => Tab::Services,
            2 => Tab::Messages,
            3 => Tab::Metrics,
            _ => Tab::Help,
        }
    }
}

/// Agent entry for display
#[derive(Debug, Clone)]
struct AgentEntry {
    name: String,
    status: String,
    node: String,
}

/// Service entry for display
#[derive(Debug, Clone)]
struct ServiceEntry {
    name: String,
    provider: String,
    protocol: String,
}

/// Message entry for display
#[derive(Debug, Clone)]
struct MessageEntry {
    timestamp: String,
    from: String,
    to: String,
    performative: String,
    content_preview: String,
}

/// Metrics data
#[derive(Debug, Clone, Default)]
struct MetricsData {
    healthy: bool,
    status: String,
    node_id: String,
    active_agents: u32,
    messages_sent: u64,
    messages_received: u64,
    cpu_usage: f32,
    memory_used: u64,
    memory_available: u64,
}

/// Application state
struct App {
    /// Current tab
    current_tab: Tab,

    /// Connection state
    connected: bool,
    connection_error: Option<String>,

    /// Node address
    node_address: String,

    /// gRPC client (wrapped in Option for lazy init)
    client: Option<FipaAgentServiceClient<Channel>>,

    /// Agent list state
    agent_list_state: ListState,
    agents: Vec<AgentEntry>,

    /// Service list state
    service_list_state: ListState,
    services: Vec<ServiceEntry>,

    /// Message list state
    message_list_state: ListState,
    messages: Vec<MessageEntry>,

    /// Metrics
    metrics: MetricsData,

    /// Last refresh time
    last_refresh: Instant,

    /// Refresh interval
    refresh_interval: Duration,

    /// Should quit
    should_quit: bool,

    /// Status message
    status_message: String,
}

impl App {
    fn new(node_address: String, refresh_interval: Duration) -> Self {
        Self {
            current_tab: Tab::Agents,
            connected: false,
            connection_error: None,
            node_address,
            client: None,
            agent_list_state: ListState::default(),
            agents: vec![],
            service_list_state: ListState::default(),
            services: vec![],
            message_list_state: ListState::default(),
            messages: vec![],
            metrics: MetricsData::default(),
            last_refresh: Instant::now() - Duration::from_secs(10), // Force initial refresh
            refresh_interval,
            should_quit: false,
            status_message: "Press 'h' for help, 'q' to quit".to_string(),
        }
    }

    fn next_tab(&mut self) {
        let next = (self.current_tab.index() + 1) % Tab::titles().len();
        self.current_tab = Tab::from_index(next);
    }

    fn prev_tab(&mut self) {
        let prev = if self.current_tab.index() == 0 {
            Tab::titles().len() - 1
        } else {
            self.current_tab.index() - 1
        };
        self.current_tab = Tab::from_index(prev);
    }

    fn select_next(&mut self) {
        match self.current_tab {
            Tab::Agents => {
                let i = match self.agent_list_state.selected() {
                    Some(i) => {
                        if i >= self.agents.len().saturating_sub(1) {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.agent_list_state.select(Some(i));
            }
            Tab::Services => {
                let i = match self.service_list_state.selected() {
                    Some(i) => {
                        if i >= self.services.len().saturating_sub(1) {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.service_list_state.select(Some(i));
            }
            Tab::Messages => {
                let i = match self.message_list_state.selected() {
                    Some(i) => {
                        if i >= self.messages.len().saturating_sub(1) {
                            0
                        } else {
                            i + 1
                        }
                    }
                    None => 0,
                };
                self.message_list_state.select(Some(i));
            }
            _ => {}
        }
    }

    fn select_prev(&mut self) {
        match self.current_tab {
            Tab::Agents => {
                let i = match self.agent_list_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.agents.len().saturating_sub(1)
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.agent_list_state.select(Some(i));
            }
            Tab::Services => {
                let i = match self.service_list_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.services.len().saturating_sub(1)
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.service_list_state.select(Some(i));
            }
            Tab::Messages => {
                let i = match self.message_list_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            self.messages.len().saturating_sub(1)
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.message_list_state.select(Some(i));
            }
            _ => {}
        }
    }

    async fn refresh_data(&mut self) {
        if self.last_refresh.elapsed() < self.refresh_interval {
            return;
        }

        self.last_refresh = Instant::now();

        // Try to connect if not connected
        if self.client.is_none() {
            match Channel::from_shared(self.node_address.clone()) {
                Ok(endpoint) => {
                    match endpoint
                        .timeout(Duration::from_secs(5))
                        .connect()
                        .await
                    {
                        Ok(channel) => {
                            self.client = Some(FipaAgentServiceClient::new(channel));
                            self.connected = true;
                            self.connection_error = None;
                            self.status_message = format!("Connected to {}", self.node_address);
                        }
                        Err(e) => {
                            self.connected = false;
                            self.connection_error = Some(format!("Connection failed: {}", e));
                            self.status_message = format!("Connection failed: {}", e);
                            return;
                        }
                    }
                }
                Err(e) => {
                    self.connected = false;
                    self.connection_error = Some(format!("Invalid endpoint: {}", e));
                    return;
                }
            }
        }

        if let Some(client) = &mut self.client {
            // Fetch health/metrics
            if let Ok(response) = client
                .health_check(HealthCheckRequest {
                    include_metrics: true,
                })
                .await
            {
                let health = response.into_inner();
                self.metrics.healthy = health.healthy;
                self.metrics.status = health.status;

                if let Some(m) = health.metrics {
                    self.metrics.active_agents = m.active_agents;
                    self.metrics.messages_sent = m.messages_sent;
                    self.metrics.messages_received = m.messages_received;
                    self.metrics.cpu_usage = m.cpu_usage_percent;
                    self.metrics.memory_used = m.memory_used_bytes;
                    self.metrics.memory_available = m.memory_available_bytes;
                }
            }

            // Fetch node info
            if let Ok(response) = client.get_node_info(NodeInfoRequest {}).await {
                let info = response.into_inner();
                self.metrics.node_id = info.node_id;
            }

            // Fetch services
            if let Ok(response) = client
                .find_service(FindServiceRequest {
                    service_name: String::new(),
                    required_protocol: None,
                    ontology: None,
                    max_results: 100,
                })
                .await
            {
                let resp = response.into_inner();
                self.services = resp
                    .providers
                    .iter()
                    .map(|p| ServiceEntry {
                        name: p
                            .service
                            .as_ref()
                            .map(|s| s.name.clone())
                            .unwrap_or_default(),
                        provider: p
                            .agent_id
                            .as_ref()
                            .map(|a| a.name.clone())
                            .unwrap_or_default(),
                        protocol: "request".to_string(),
                    })
                    .collect();
            }

            // Add some sample agents (in real implementation, would use ListAgents RPC)
            if self.agents.is_empty() {
                self.agents = vec![
                    AgentEntry {
                        name: "ams".to_string(),
                        status: "running".to_string(),
                        node: self.metrics.node_id.clone(),
                    },
                    AgentEntry {
                        name: "df".to_string(),
                        status: "running".to_string(),
                        node: self.metrics.node_id.clone(),
                    },
                ];
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(args.node, Duration::from_millis(args.refresh));

    // Main loop
    let result = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Refresh data periodically
        app.refresh_data().await;

        // Draw UI
        terminal.draw(|f| ui(f, app))?;

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => app.should_quit = true,
                        KeyCode::Char('h') => app.current_tab = Tab::Help,
                        KeyCode::Tab => app.next_tab(),
                        KeyCode::BackTab => app.prev_tab(),
                        KeyCode::Char('1') => app.current_tab = Tab::Agents,
                        KeyCode::Char('2') => app.current_tab = Tab::Services,
                        KeyCode::Char('3') => app.current_tab = Tab::Messages,
                        KeyCode::Char('4') => app.current_tab = Tab::Metrics,
                        KeyCode::Char('5') => app.current_tab = Tab::Help,
                        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                        KeyCode::Char('r') => {
                            app.last_refresh = Instant::now() - Duration::from_secs(10);
                            app.status_message = "Refreshing...".to_string();
                        }
                        KeyCode::Char('c') => {
                            // Reconnect
                            app.client = None;
                            app.connected = false;
                            app.last_refresh = Instant::now() - Duration::from_secs(10);
                            app.status_message = "Reconnecting...".to_string();
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Create main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Status bar
        ])
        .split(size);

    // Draw tabs
    let tab_titles: Vec<Line> = Tab::titles()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let style = if i == app.current_tab.index() {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            Line::from(Span::styled(format!(" {} [{}] ", t, i + 1), style))
        })
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" FIPA Agent Platform "),
        )
        .select(app.current_tab.index())
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow));

    f.render_widget(tabs, chunks[0]);

    // Draw content based on current tab
    match app.current_tab {
        Tab::Agents => draw_agents_tab(f, app, chunks[1]),
        Tab::Services => draw_services_tab(f, app, chunks[1]),
        Tab::Messages => draw_messages_tab(f, app, chunks[1]),
        Tab::Metrics => draw_metrics_tab(f, app, chunks[1]),
        Tab::Help => draw_help_tab(f, chunks[1]),
    }

    // Draw status bar
    let connection_status = if app.connected {
        Span::styled(" CONNECTED ", Style::default().fg(Color::Green))
    } else {
        Span::styled(" DISCONNECTED ", Style::default().fg(Color::Red))
    };

    let status = Paragraph::new(Line::from(vec![
        connection_status,
        Span::raw(" | "),
        Span::styled(&app.node_address, Style::default().fg(Color::Cyan)),
        Span::raw(" | "),
        Span::raw(&app.status_message),
    ]))
    .block(Block::default().borders(Borders::ALL));

    f.render_widget(status, chunks[2]);
}

fn draw_agents_tab(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|agent| {
            let status_style = if agent.status == "running" {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Yellow)
            };

            ListItem::new(Line::from(vec![
                Span::styled(&agent.name, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" | "),
                Span::styled(&agent.status, status_style),
                Span::raw(" | "),
                Span::styled(&agent.node, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let agents_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Agents (j/k to navigate) "),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(agents_list, area, &mut app.agent_list_state.clone());
}

fn draw_services_tab(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .services
        .iter()
        .map(|service| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    &service.name,
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled(
                    format!("by {}", service.provider),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(" | "),
                Span::styled(&service.protocol, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();

    let services_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Services (j/k to navigate) "),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(services_list, area, &mut app.service_list_state.clone());
}

fn draw_messages_tab(f: &mut Frame, app: &App, area: Rect) {
    if app.messages.is_empty() {
        let text = Paragraph::new(Text::raw(
            "\n  No messages captured yet.\n\n  Message sniffing will show real-time agent communications.\n  This feature requires a running sniffer agent.",
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Messages "),
        )
        .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
        return;
    }

    let items: Vec<ListItem> = app
        .messages
        .iter()
        .map(|msg| {
            ListItem::new(Line::from(vec![
                Span::styled(&msg.timestamp, Style::default().fg(Color::Gray)),
                Span::raw(" "),
                Span::styled(&msg.from, Style::default().fg(Color::Cyan)),
                Span::raw(" -> "),
                Span::styled(&msg.to, Style::default().fg(Color::Green)),
                Span::raw(" ["),
                Span::styled(&msg.performative, Style::default().fg(Color::Yellow)),
                Span::raw("] "),
                Span::raw(&msg.content_preview),
            ]))
        })
        .collect();

    let messages_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Messages (j/k to navigate) "),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(messages_list, area, &mut app.message_list_state.clone());
}

fn draw_metrics_tab(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left panel - Node info
    let health_indicator = if app.metrics.healthy {
        Span::styled("HEALTHY", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("UNHEALTHY", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let node_info = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("  Health: "),
            health_indicator,
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Node ID: "),
            Span::styled(&app.metrics.node_id, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("  Status: "),
            Span::styled(&app.metrics.status, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Active Agents: "),
            Span::styled(
                app.metrics.active_agents.to_string(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Node Status "),
    );

    f.render_widget(node_info, chunks[0]);

    // Right panel - Metrics
    let memory_total = app.metrics.memory_used + app.metrics.memory_available;
    let memory_percent = if memory_total > 0 {
        (app.metrics.memory_used as f64 / memory_total as f64 * 100.0) as u32
    } else {
        0
    };

    let metrics = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("  CPU Usage: "),
            Span::styled(
                format!("{:.1}%", app.metrics.cpu_usage),
                Style::default().fg(if app.metrics.cpu_usage > 80.0 {
                    Color::Red
                } else if app.metrics.cpu_usage > 50.0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Memory: "),
            Span::styled(
                format!("{} MB", app.metrics.memory_used / 1024 / 1024),
                Style::default().fg(Color::White),
            ),
            Span::raw(" / "),
            Span::styled(
                format!("{} MB", memory_total / 1024 / 1024),
                Style::default().fg(Color::Gray),
            ),
            Span::raw(format!(" ({}%)", memory_percent)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Messages Sent: "),
            Span::styled(
                app.metrics.messages_sent.to_string(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Messages Received: "),
            Span::styled(
                app.metrics.messages_received.to_string(),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Performance Metrics "),
    );

    f.render_widget(metrics, chunks[1]);
}

fn draw_help_tab(f: &mut Frame, area: Rect) {
    let help_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Navigation", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
        ]),
        Line::from("  ─────────────────────────────────────────"),
        Line::from("  Tab / Shift+Tab    Switch between tabs"),
        Line::from("  1-5                Jump to tab by number"),
        Line::from("  j / Down           Move down in list"),
        Line::from("  k / Up             Move up in list"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Actions", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
        ]),
        Line::from("  ─────────────────────────────────────────"),
        Line::from("  r                  Refresh data"),
        Line::from("  c                  Reconnect to node"),
        Line::from("  q                  Quit application"),
        Line::from("  h                  Show this help"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Tabs", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow)),
        ]),
        Line::from("  ─────────────────────────────────────────"),
        Line::from("  [1] Agents         List and manage agents"),
        Line::from("  [2] Services       Browse registered services"),
        Line::from("  [3] Messages       View message stream"),
        Line::from("  [4] Metrics        Platform performance"),
        Line::from("  [5] Help           This help screen"),
        Line::from(""),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help - Keyboard Shortcuts "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(help, area);
}
