# Behavior Demo Agent

Demonstrates JADE-style behaviors in a FIPA WASM agent.

## Behaviors Demonstrated

| Behavior | Type | Description |
|----------|------|-------------|
| `heartbeat` | Ticker | Logs a heartbeat every 5 seconds |
| `message-handler` | Cyclic | Processes incoming messages continuously |
| `startup-task` | OneShot | Runs once at startup |
| `delayed-task` | Waker | Runs once after 10 second delay |
| `state-machine` | FSM | Demonstrates state machine transitions |

## FSM States

```
     start           done
idle ────► processing ────► completed
      ◄────          ◄────
     cancel          reset
```

## Building

```bash
cargo build --release --target wasm32-wasip1
```

## Sending Messages

Send messages containing these keywords to trigger FSM transitions:
- `start` - Transition from idle to processing
- `done` - Transition from processing to completed
- `cancel` - Transition from processing to idle
- `reset` - Transition from completed to idle

## Example Output

```
[INFO] Behavior Demo Agent 'demo-agent' initializing...
[INFO] Registered heartbeat behavior (id=1)
[INFO] Registered message handler (id=2)
[INFO] Registered startup task (id=3)
[INFO] Registered delayed task (id=4)
[INFO] Registered FSM behavior (id=5)
[INFO] All behaviors registered. Agent ready!
[INFO] 🚀 Startup task executing (one-shot)
[INFO] 💓 Heartbeat #1
[INFO] ⏰ Delayed task woke up after 10 seconds!
[INFO] 💓 Heartbeat #2
...
```
