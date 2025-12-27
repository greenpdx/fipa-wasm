# Calculator Agent

A FIPA agent demonstrating service registration and the Query interaction protocol.

## Features

- Registers as a "calculator" service on startup
- Handles QUERY-REF messages with math expressions
- Returns INFORM-REF with calculated results
- Supports basic operations: `+`, `-`, `*`, `/`, `%`, `^`
- Supports parentheses for grouping

## Service Registration

On startup, the agent registers itself with:
- **Name**: `calculator`
- **Description**: Mathematical expression evaluator
- **Protocols**: Query
- **Ontology**: `math`

Other agents can discover this service using:
```
find-agents-by-service("calculator")
```

## Usage

Send a QUERY-REF message with the math expression as content:

```
-> QUERY-REF: "2 + 3"
<- INFORM-REF: "5"

-> QUERY-REF: "10 * 5 - 3"
<- INFORM-REF: "47"

-> QUERY-REF: "2 ^ 8"
<- INFORM-REF: "256"

-> QUERY-REF: "(3 + 4) * (2 + 1)"
<- INFORM-REF: "21"

-> QUERY-REF: "10 / 0"
<- FAILURE: "Error: Division by zero"
```

## FIPA Compliance

Follows FIPA Query Interaction Protocol (FIPA00027):

```
Initiator                      Participant (calculator)
    |                                |
    |  QUERY-REF (expression)        |
    |------------------------------->|
    |                                |
    |  INFORM-REF (result)           |
    |<-------------------------------|
    |                                |
```

Or on error:

```
    |  QUERY-REF (bad expr)          |
    |------------------------------->|
    |                                |
    |         FAILURE (error)        |
    |<-------------------------------|
```

## Building

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
```

## Supported Operations

| Operator | Description | Example |
|----------|-------------|---------|
| `+` | Addition | `3 + 5` → `8` |
| `-` | Subtraction | `10 - 3` → `7` |
| `*` | Multiplication | `4 * 5` → `20` |
| `/` | Division | `15 / 3` → `5` |
| `%` | Modulo | `17 % 5` → `2` |
| `^` | Power | `2 ^ 10` → `1024` |
| `()` | Grouping | `(2+3)*4` → `20` |

## License

MIT
