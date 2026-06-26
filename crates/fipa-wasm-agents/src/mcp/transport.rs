// mcp/transport.rs - MCP Transport Layer
//
//! Transport implementations for MCP communication.

use super::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use super::McpError;
use std::io::{BufRead, Write};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tracing::{debug, error, trace};

/// Message from transport
#[derive(Debug)]
pub enum TransportMessage {
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

/// Stdio transport for MCP
pub struct StdioTransport {
    tx: mpsc::Sender<String>,
}

impl StdioTransport {
    /// Create a new stdio transport and start reading
    pub fn new() -> (Self, mpsc::Receiver<TransportMessage>) {
        let (msg_tx, msg_rx) = mpsc::channel(100);
        let (out_tx, mut out_rx) = mpsc::channel::<String>(100);

        // Spawn reader task
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let mut reader = BufReader::new(stdin);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        debug!("EOF on stdin, shutting down");
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        trace!("Received: {}", trimmed);

                        // Try to parse as request or notification
                        if let Ok(req) = serde_json::from_str::<JsonRpcRequest>(trimmed) {
                            if msg_tx.send(TransportMessage::Request(req)).await.is_err() {
                                break;
                            }
                        } else if let Ok(notif) =
                            serde_json::from_str::<JsonRpcNotification>(trimmed)
                        {
                            if msg_tx.send(TransportMessage::Notification(notif)).await.is_err() {
                                break;
                            }
                        } else {
                            error!("Failed to parse message: {}", trimmed);
                        }
                    }
                    Err(e) => {
                        error!("Error reading stdin: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn writer task
        tokio::spawn(async move {
            let mut stdout = tokio::io::stdout();

            while let Some(msg) = out_rx.recv().await {
                trace!("Sending: {}", msg);
                if let Err(e) = stdout.write_all(msg.as_bytes()).await {
                    error!("Error writing to stdout: {}", e);
                    break;
                }
                if let Err(e) = stdout.write_all(b"\n").await {
                    error!("Error writing newline: {}", e);
                    break;
                }
                if let Err(e) = stdout.flush().await {
                    error!("Error flushing stdout: {}", e);
                    break;
                }
            }
        });

        (Self { tx: out_tx }, msg_rx)
    }

    /// Send a response
    pub async fn send_response(&self, response: JsonRpcResponse) -> Result<(), McpError> {
        let json = serde_json::to_string(&response)?;
        self.tx
            .send(json)
            .await
            .map_err(|e| McpError::Transport(e.to_string()))
    }

    /// Send a notification
    pub async fn send_notification(&self, notification: JsonRpcNotification) -> Result<(), McpError> {
        let json = serde_json::to_string(&notification)?;
        self.tx
            .send(json)
            .await
            .map_err(|e| McpError::Transport(e.to_string()))
    }
}

/// Synchronous stdio transport (for simpler use cases)
pub struct SyncStdioTransport;

impl SyncStdioTransport {
    /// Read a single message from stdin
    pub fn read_message() -> Result<TransportMessage, McpError> {
        let stdin = std::io::stdin();
        let mut line = String::new();

        stdin
            .lock()
            .read_line(&mut line)
            .map_err(|e| McpError::Transport(e.to_string()))?;

        let trimmed = line.trim();

        if let Ok(req) = serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(TransportMessage::Request(req))
        } else if let Ok(notif) = serde_json::from_str::<JsonRpcNotification>(trimmed) {
            Ok(TransportMessage::Notification(notif))
        } else {
            Err(McpError::JsonRpc(format!("Invalid message: {}", trimmed)))
        }
    }

    /// Write a response to stdout
    pub fn write_response(response: &JsonRpcResponse) -> Result<(), McpError> {
        let json = serde_json::to_string(response)?;
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json).map_err(|e| McpError::Transport(e.to_string()))?;
        stdout
            .flush()
            .map_err(|e| McpError::Transport(e.to_string()))
    }

    /// Write a notification to stdout
    pub fn write_notification(notification: &JsonRpcNotification) -> Result<(), McpError> {
        let json = serde_json::to_string(notification)?;
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", json).map_err(|e| McpError::Transport(e.to_string()))?;
        stdout
            .flush()
            .map_err(|e| McpError::Transport(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_message_parsing() {
        let req_json = r#"{"jsonrpc":"2.0","id":1,"method":"test","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(req_json).unwrap();
        assert_eq!(req.method, "test");

        let notif_json = r#"{"jsonrpc":"2.0","method":"notify","params":{}}"#;
        let notif: JsonRpcNotification = serde_json::from_str(notif_json).unwrap();
        assert_eq!(notif.method, "notify");
    }
}
