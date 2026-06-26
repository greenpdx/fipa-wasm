# fipa workspace

A Cargo workspace holding two related product lines:

| Crate | What it is |
|-------|-----------|
| [`crates/fipa-wasm-agents`](crates/fipa-wasm-agents/) | FIPA WASM distributed agent system — actor model, libp2p, Raft consensus. |
| [`crates/unl-core`](crates/unl-core/) | Foundational types for the open Rust **UNL** (Universal Networking Language) stack. |

The UNL stack is being built out per the design spec in `~/SOURCES_MANIFEST.md`.
UNL is positioned as the formally-verifiable *content language* that slots under
a FIPA ACL performative envelope, connecting the two lines.

## Layout

```
Cargo.toml            # workspace manifest (members, shared deps, profiles, patches)
crates/
  fipa-wasm-agents/   # the FIPA agent system (its own README, build.rs, benches)
  unl-core/           # UNL hypergraph types: Uw, Relation, Attr, UnlGraph, ...
data/
  relations.toml      # the universal relation set + hierarchy (mirrors unl-core)
  attributes.toml     # the universal attribute families (mirrors unl-core)
  raft/               # fipa-node runtime state
```

## Building

```sh
cargo test -p unl-core                       # UNL core types (fast, light deps)
cargo build -p fipa-wasm-agents --bin fipa-node   # the agent node (heavy deps)
```

`unl-core` is dependency-light (serde + small helpers, no async, no I/O), so it
builds and tests in isolation without pulling in the agent system's toolchain.
