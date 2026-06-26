# Counter Agent

A FIPA agent demonstrating persistent storage. The counter value survives
agent restarts and migrations.

## Commands

Send REQUEST messages with these commands in the content:

| Command | Description |
|---------|-------------|
| `increment`, `inc`, `++` | Add 1 to counter |
| `decrement`, `dec`, `--` | Subtract 1 (min 0) |
| `get`, `value`, `?` | Get current value |
| `reset`, `clear` | Reset to 0 |
| `add:N` | Add N (can be negative) |
| `set:N` | Set to specific value |

## Response Format

INFORM message with content: `<value>:<description>`

Example: `42:Counter incremented to 42`

## Building

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

## Example Interaction

```
-> REQUEST: "get"
<- INFORM: "0:Counter value is 0"

-> REQUEST: "add:10"
<- INFORM: "10:Added 10, counter is now 10"

-> REQUEST: "inc"
<- INFORM: "11:Counter incremented to 11"

-> REQUEST: "dec"
<- INFORM: "10:Counter decremented to 10"
```

## Persistence

The counter value is stored using the FIPA storage interface:
- Key: `counter_value`
- Format: 8-byte little-endian u64

This persists across:
- Agent restarts
- Agent migrations to other nodes
- Node restarts (if storage backend persists)

## License

MIT
