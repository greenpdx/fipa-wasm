// observability/tracing_setup.rs - Tracing Configuration

use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

/// Tracing output format
#[derive(Clone, Debug, Default)]
pub enum TracingFormat {
    /// Human-readable format (default)
    #[default]
    Pretty,
    /// Compact single-line format
    Compact,
    /// JSON format for log aggregation
    Json,
}

/// Configuration for tracing
#[derive(Clone, Debug)]
pub struct TracingConfig {
    /// Log level filter (e.g., "info", "debug", "fipa_wasm_agents=debug")
    pub filter: String,

    /// Output format
    pub format: TracingFormat,

    /// Include span events (new, close)
    pub with_span_events: bool,

    /// Include file and line numbers
    pub with_file: bool,

    /// Include target (module path)
    pub with_target: bool,

    /// Include thread IDs
    pub with_thread_ids: bool,

    /// Include thread names
    pub with_thread_names: bool,

    /// ANSI colors (for terminal output)
    pub with_ansi: bool,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            filter: "info,fipa_wasm_agents=debug".into(),
            format: TracingFormat::Pretty,
            with_span_events: false,
            with_file: false,
            with_target: true,
            with_thread_ids: false,
            with_thread_names: false,
            with_ansi: true,
        }
    }
}

impl TracingConfig {
    /// Create a production config (JSON, minimal overhead)
    pub fn production() -> Self {
        Self {
            filter: "info,fipa_wasm_agents=info".into(),
            format: TracingFormat::Json,
            with_span_events: false,
            with_file: false,
            with_target: true,
            with_thread_ids: false,
            with_thread_names: false,
            with_ansi: false,
        }
    }

    /// Create a development config (pretty, verbose)
    pub fn development() -> Self {
        Self {
            filter: "debug,fipa_wasm_agents=trace".into(),
            format: TracingFormat::Pretty,
            with_span_events: true,
            with_file: true,
            with_target: true,
            with_thread_ids: true,
            with_thread_names: false,
            with_ansi: true,
        }
    }
}

/// Initialize the tracing subscriber
///
/// This should be called once at application startup.
pub fn init_tracing(config: TracingConfig) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.filter));

    let span_events = if config.with_span_events {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    match config.format {
        TracingFormat::Pretty => {
            let fmt_layer = fmt::layer()
                .with_span_events(span_events)
                .with_file(config.with_file)
                .with_line_number(config.with_file)
                .with_target(config.with_target)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names)
                .with_ansi(config.with_ansi);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
        TracingFormat::Compact => {
            let fmt_layer = fmt::layer()
                .compact()
                .with_span_events(span_events)
                .with_file(config.with_file)
                .with_line_number(config.with_file)
                .with_target(config.with_target)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names)
                .with_ansi(config.with_ansi);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
        TracingFormat::Json => {
            let fmt_layer = fmt::layer()
                .json()
                .with_span_events(span_events)
                .with_file(config.with_file)
                .with_line_number(config.with_file)
                .with_target(config.with_target)
                .with_thread_ids(config.with_thread_ids)
                .with_thread_names(config.with_thread_names);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
    }

    tracing::info!(
        filter = %config.filter,
        format = ?config.format,
        "Tracing initialized"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_config_default() {
        let config = TracingConfig::default();
        assert!(config.filter.contains("info"));
        assert!(config.with_ansi);
    }

    #[test]
    fn test_tracing_config_production() {
        let config = TracingConfig::production();
        assert!(matches!(config.format, TracingFormat::Json));
        assert!(!config.with_ansi);
    }
}
