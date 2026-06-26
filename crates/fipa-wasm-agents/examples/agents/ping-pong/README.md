# Ping-Pong Agent

A simple FIPA agent that demonstrates basic messaging patterns.

## Behavior

- Listens for REQUEST messages containing "ping"
- Responds with INFORM messages containing "pong"
- Uses the FIPA Request protocol
- Tracks message statistics

## Building

```bash
# Add WASM target (first time only)
rustup target add wasm32-wasip2

# Build the agent
cargo build --release --target wasm32-wasip2
```

The compiled WASM module will be at:
`target/wasm32-wasip2/release/ping_pong_agent.wasm`

## Testing

Send a ping message to this agent:

```json
{
  "performative": "REQUEST",
  "content": "ping",
  "protocol": "request"
}
```

You should receive:

```json
{
  "performative": "INFORM",
  "content": "pong",
  "protocol": "request",
  "in_reply_to": "<original_message_id>"
}
```

## FIPA Compliance

This agent follows the FIPA Request Interaction Protocol (FIPA00026):

```
Initiator                      Participant (this agent)
    |                                |
    |  REQUEST (ping)                |
    |------------------------------->|
    |                                |
    |         INFORM (pong)          |
    |<-------------------------------|
    |                                |
```

## Code Structure

- `init()` - Logs startup message
- `run()` - Processes incoming messages in a loop
- `shutdown()` - Logs statistics
- `handle_message_internal()` - Routes messages by performative
- `send_pong()` - Constructs and sends pong reply

## License

MIT
