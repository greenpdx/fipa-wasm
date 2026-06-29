# FIPA-UNL Agent System — Architecture

**Version:** 0.3.0
**Last Updated:** 2026-06-29
**Author:** Shaun Savage (SavageS)
**Companion spec:** [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md) — the detailed agent↔host contract.

> **Reading note.** This document describes the system as it is **actually built**.
> The agent↔host boundary, the security gates, the capabilities, scheduling,
> migration, and the secured cross-node transport are all implemented and tested on
> `main`; the small remainder that is still designed-only is stated explicitly in
> [§12 Status](#12-status-built-vs-planned).
> An earlier draft of this file described a libp2p/Raft/Actix/WASI-P2 platform that
> was never implemented; that vision is preserved, clearly labelled as aspirational,
> in [Appendix A](#appendix-a-future-directions-not-built).

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Governing principles](#2-governing-principles)
3. [System overview](#3-system-overview)
4. [The node as resource manager](#4-the-node-as-resource-manager)
5. [Agents are block bundles](#5-agents-are-block-bundles)
6. [Identity](#6-identity)
7. [The message model](#7-the-message-model)
8. [Discovery and the platform agents](#8-discovery-and-the-platform-agents)
9. [Node initialization](#9-node-initialization)
10. [Security model](#10-security-model)
11. [The three brains — one seam](#11-the-three-brains--one-seam)
12. [Status: built vs planned](#12-status-built-vs-planned)
13. [Glossary](#13-glossary)
- [Appendix A: Future directions (not built)](#appendix-a-future-directions-not-built)

---

## 1. Executive summary

This is a [FIPA](http://www.fipa.org/)-aligned multi-agent platform whose agents
carry semantic content as [UNL](http://www.unlweb.net/) (Universal Networking
Language) graphs. It is built around two ideas:

1. **The agent is small.** An agent is a *passive bundle of blocks* — code,
   vocabulary, optional model, data, state — that computes nothing on its own.
2. **The node gates everything.** A trusted **Rust** node loads the agent, *runs*
   its brain, and mediates every byte in and out. The node is the **reference
   monitor**; the agent has no ambient authority.

A running example — a book-buying conversation — exercises the whole platform: a
Buyer (BA) discovers a book seller through the **DF** (yellow pages), resolves its
address through the **AMS** (white pages), buys *Limits to Growth* through a
Payment Agent (**PA**) escrow, and the seller ships. This runs across five nodes
(one agent each) over TCP/IP, in Docker containers by IP, today.

---

## 2. Governing principles

| # | Principle | Consequence |
|---|---|---|
| P1 | **Small agent, gating node** | The node is the only trust boundary; agents are sandboxed and capability-bounded. |
| P2 | **The node is Rust, native, trusted** | The reference monitor cannot be a sandboxed wasm agent — it *is* the sandbox. |
| P3 | **Least privilege, declared in the manifest** | The agent's `HEAD` block is read **first, always**; it declares which capabilities the agent may use. The node grants nothing else. |
| P4 | **wasm-first for mobility** | wasm agents run on every node shape (including IoT and, in future, the browser). Native Rust is reserved for big, stationary agents. |
| P5 | **OS/transport-agnostic ABI** | The agent↔host ABI assumes no filesystem and no sockets — only host calls — so a browser node can implement the same contract. |
| P6 | **Log rich, reply thin** | Every gate logs maximal forensic detail node-side and returns a single uniform `denied` agent-side. The asymmetry is absolute. |
| P7 | **UNL for human/semantic content, JSON for structured machine data** | UNL graphs carry meaning (services, requests); JSON bodies carry UUIDs, amounts, addresses. UUIDs never appear as UNL words. |

---

## 3. System overview

```
            ┌──────────────────────── NODE (Rust, trusted reference monitor) ───────────────────────┐
            │                                                                                        │
 inbound    │   IN-GATE                         BRAIN                       OUT-GATE                  │  outbound
 ─────────► │  decode · authenticate ─►  wasm │ native │ llm  ─►  validate · scope · rate  ─────────►│ ─────────►
 (from,     │  rate-limit                  (runs the agent's blocks)        (OutboundIntent)          │  (from,
  unl,body) │       │                            │                              │                     │   unl,body)
            │       └──── audit log (rich) ──────┴──────── audit log (rich) ────┘                     │
            │                                                                                        │
            │   PLATFORM AGENTS (native, trusted):  AMS · DF · [PA …]   ── back the discovery host-calls
            │   TENANT AGENTS  (bundle-loaded, gated):  BA · BS · …                                  │
            └────────────────────────────────────────────────────────────────────────────────────────┘
                                                   │ TCP/IP (today)  ·  browser channel (future)
                                            ┌──────┴───────┐
                                            │  Other nodes │
                                            └──────────────┘
```

- A **node** hosts one or more agents, runs a small **platform** (AMS/DF/PA),
  and routes messages to other nodes.
- An **agent** is a bundle of blocks the node runs behind a single seam
  (`AgentRuntime`). It reacts to messages and timer ticks; it emits messages and
  gated host-calls — all of which pass through the node's gates.
- Today's cross-node transport is a hand-rolled length-prefixed **TCP** protocol
  (`process::node`). One agent per node maps cleanly onto one container per agent.

---

## 4. The node as resource manager

The node — **not** any agent — is the resource manager: it loads agents, implements
and gates the ABI, routes communication, and runs the platform agents. It is always
**Rust source**. What varies per deployment is the embedded wasm engine, the ABI
breadth, and the budgets:

| | **normal node** | **IoT node** | **browser node** (future) |
|---|---|---|---|
| resource manager | Rust binary | Rust binary (small) | Rust → wasm, in-page |
| wasm engine for agents | JIT (e.g. wasmtime) | small **interpreter** (e.g. wasmi) | the browser's wasm engine |
| ABI breadth | full | limited (load-time fit) | full-ish (no FS) |
| platform agents | AMS · DF · PA (+) | AMS · DF (minimal) | AMS · DF |
| budgets | MB / megafuel | KB / kilofuel | tab limits |

The same Rust node is the same reference monitor everywhere; only its *engine* and
*ABI surface* shrink for constrained targets. This is why P5 (OS/transport-agnostic
ABI) matters: the browser node is literally the Rust node compiled to wasm, hosting
agent wasm modules.

---

## 5. Agents are block bundles

An agent is a passive bundle. The node runs and gates each block.

| Block | Contents | Who runs it |
|---|---|---|
| **HEAD** | the **manifest** — identity, profile, brain kind, grants, budgets | node reads it **first, always** |
| **WASM** | agent code (optional) | node's wasm engine |
| **UNL** | the agent's vocabulary / system frame (decoded) | node decodes; feeds the brain |
| **LLM** | model bytes **or** a reference (`ollama:…`, model id, endpoint) | node runs via `ReasoningBackend` |
| **DATA** | static seed | node → `seed()` |
| **STATE** | mutable durable state | node-owned, agent-scoped, quota'd |
| **SIG** | signature over the bundle | node verifies before load |

A given agent uses *some* blocks: the Buyer is `HEAD+WASM`; an LLM agent is
`HEAD+UNL+LLM(+STATE)`; an infrastructure agent is `HEAD+native(+STATE)`.

The manifest is the **resource-management record** for the agent. See
[`AGENT_HOST_ABI.md` §4](./AGENT_HOST_ABI.md) for its schema and the load sequence.

---

## 6. Identity

Identity follows the FIPA **Agent Identifier (AID)** — `{ name, addresses,
resolvers }` — with `name` made a **UUID** so identity is location-independent and
survives migration (addresses are resolved via AMS, never baked in).

- **instance UUID** — the AID `name`, unique per *running* agent.
- **type `{UUID, desc}`** — the *kind* of agent, carried in `HEAD`; many instances
  share a type.
- **friendly name** — display/logs only, never the identity.

Lifecycle:

- **ephemeral / mobile** agents **mint** a fresh instance UUID at spawn
  (`AgentId::spawn`);
- **persistent infrastructure** agents **persist** their UUID
  (`AgentId::load_or_mint`) so long-lived references (a held order naming the
  seller) survive a restart.

A UUID is *structured machine data*: it travels as a routing key (`from`/`to`) and
inside **JSON bodies**, never as a UNL word (P7). Implementation: `identity.rs`
(`AgentType`, `Header`, `AgentId`, `Aliases`).

---

## 7. The message model

Every message is a triple:

```
(from, unl, body)
```

- **`from`** — the sender's instance UUID. It is **authenticated**: the node stamps
  it from its own knowledge of the emitter; the agent never sets it. Intra-node,
  the `Router` stamps `from` from the resolved sender (an `OutboundIntent` has no
  sender field, so it is unforgeable). Cross-node, every `NodeMsg` is **signed by
  the sending node's ed25519 key** and verified on receipt, so a remote `from` is
  authenticated too (built — see [§12](#12-status-built-vs-planned)).
- **`unl`** — a UNL graph: the **human/semantic** content (a service name, a
  request, an offer). This is what would "come from people."
- **`body`** — JSON: **structured machine data** (UUIDs, amounts, addresses).

This split (P7) keeps UNL purely semantic and keeps machine identifiers out of the
human content. FIPA ACL envelope fields (performative, conversation-id,
in-reply-to, reply-by, protocol) are an **opt-in** layer on top — see the ABI spec.

---

## 8. Discovery and the platform agents

Two FIPA system agents provide discovery; a third provides escrow:

- **AMS** (Agent Management System) — **white pages**: UUID → address. The agent
  UUID is carried in the JSON body, not in UNL.
- **DF** (Directory Facilitator) — **yellow pages**: a service (a UNL graph) →
  provider UUIDs (in the JSON body; the provider is the authenticated `from`).
- **PA** (Payment Agent) — escrow: `reserve` / `accept` / `deny` holds over a
  durable ledger (sled), so a buyer and seller can transact without trusting each
  other.

**Discovery is being promoted to typed, gated, async host-calls** (`find_service`,
`locate`). These are a thin *façade*: the node satisfies them by routing to its own
AMS/DF platform agents. Promotion does not replace AMS/DF — it puts a typed,
least-privilege front door on them. (Today, agents reach AMS/DF by *sending them
messages* directly; the host-call façade is planned — see [§12](#12-status-built-vs-planned).)

---

## 9. Node initialization

A node boots a small **platform** *before* any tenant agent, because the platform is
what *backs* the discovery host-calls:

```
NODE BOOT
 1. load node config + node keypair/UUID        (the node has its own identity, for attestation)
 2. select profile:  normal | iot
 3. bring up PLATFORM agents in-process (native, trusted):
        AMS (white pages) · DF (yellow pages) · [PA, … on a full node]
        ── an IoT node ALSO loads AMS + DF, minimal ──  (so a local agent's find_service has an answer, even offline)
 4. wire routes (in-process for co-located AMS/DF; bootstrap addresses for remote peers)
 5. open transport (TCP today; browser channel later — same ABI)
 6. register node/self with the platform
 ── platform is now live ──
 7. load TENANT agents from bundles:
        read HEAD ─► verify SIG ─► gate (profile fit + grants) ─► instantiate brain ─► init() ─► seed()
```

Consequence: an **IoT node is a self-contained mini-platform** — it carries its own
AMS+DF so it works offline. Two agent classes result: **platform agents** (native,
trusted, *back* the host-calls) and **tenant agents** (bundle-loaded, gated,
*consume* them).

---

## 10. Security model

The security model is the reference monitor (P1/P2) plus least privilege (P3) plus
the audit asymmetry (P6).

### 10.1 The gates

Four gates, all node-side:

1. **Load-time fit** — the node reads `HEAD` and matches declared `grants` against
   the node profile. A wasm agent that needs `llm` is rejected at the door of an
   IoT node, **before it runs**, reported to the *operator*. This is where a node
   may genuinely link a *smaller* ABI (footprint) without the agent probing.
2. **In-gate** — inbound `(from, unl, body)`: decode, authenticate `from`,
   rate-limit, authorize delivery.
3. **Out-gate** — every emitted message and host-call (`OutboundIntent`) is
   validated, network-scoped, and rate-limited before it takes effect.
4. **Runtime budget** — memory, fuel/CPU, storage quota, timer-slot count, message
   rate. (wasm runs under hard per-call **fuel + memory limits**; native agents run
   under `catch_unwind` fault isolation, and the supervisor quarantines a faulting
   agent — all built. See [§12](#12-status-built-vs-planned).)

### 10.2 Uniform denials, no info leak

Once an agent is admitted, **any** disallowed call returns the **same opaque
`denied`** — whether the capability is absent-by-profile or ungranted-by-manifest.
The agent cannot distinguish the reasons; it cannot probe the host. Load-time fit is
operator-facing and trusted; runtime denial is agent-facing and opaque. Both hold at
once because they happen at different times.

### 10.3 Log rich, reply thin (P6)

| | audit/log channel (node-side, trusted) | agent reply (untrusted) |
|---|---|---|
| permission violation | agent UUID+type, node id, capability, exact call+args (or hashes), the rule that denied it, manifest grants, timestamp, correlation id | a single uniform `denied` |
| failure (panic/trap/quota) | brain kind, fuel/mem at fault, panic payload or wasm trap, state ref | uniform terminal status; output discarded |

The log is node-attributed and agent-unspoofable: an agent cannot forge or suppress
its own violation record. A probing agent that tries every capability gets back an
undifferentiated wall of `denied` — while the audit log captures the entire probe,
which is itself the strongest abuse signal.

### 10.4 Sandbox tiers

- **wasm** — memory isolation by construction; resource caps via engine fuel/memory
  limits (built); host access only through granted imports.
- **native** — agent crates set `#![forbid(unsafe_code)]`; every call runs under
  `catch_unwind` so a panic is contained and the agent quarantined, not the node.
  Hard CPU/RAM caps need a thread/process boundary (the remaining native upgrade).

### 10.5 Key custody and crypto

Cryptographic private keys live **node-side**, in the keystore — never in an agent's
sandbox. The `crypto` capability exposes operations (`sign`/`verify`/`random`), not
secrets, so the node acts as a **signing oracle**: an agent compromise cannot leak a
key. This is faster (native constant-time ed25519, one vetted implementation) and
consistent with the node already holding LLM keys and cost.

The oracle's one risk — an agent coaxing it to sign bytes meaningful elsewhere (a
confused deputy) — is closed by **domain separation**: the node decides *what* is
signed (it builds and signs message envelopes itself; the agent supplies only
content), uses **per-purpose keys + domain tags**, and rate-limits and audits every
`sign`. `random` is necessarily node-provided (wasm has no entropy source). Full
treatment: [`AGENT_HOST_ABI.md` §7.2](./AGENT_HOST_ABI.md#72-crypto-key-custody-and-domain-separation).

---

## 11. The three brains — one seam

Three kinds of brain present the **same** lifecycle to the node, through the
`AgentRuntime` seam (`init` / `config(from,unl,body)` / `take_sends` / `run` /
`shutdown`). The rest of the node is brain-agnostic:

| Brain | Runtime | Use | Status |
|---|---|---|---|
| **wasm** | `WasmRuntime` (wasmtime JIT + wasmi interpreter, per profile) | mobile / untrusted / portable agents | built |
| **native** | `NativeRuntime<A>` | big, stationary agents (AMS/DF/PA) | built |
| **llm** | `LlmRuntime` / the `llm` capability | the node runs the agent's LLM block as its brain | built (as the `llm` capability) |

`LlmRuntime` maps `config(from,unl,body)` → a `Prompt` built from (UNL system frame
+ STATE + inbound) → `unl_llm::ReasoningBackend::complete` → parsed into
`OutboundIntent`s + state writes. This is literally "the LLM model or reference that
the node runs," reusing `unl-llm` wholesale.

---

## 12. Status: built vs planned

All of the following is on `main`, tested, with the 5-node `book-cluster`
verifying `obj(bought, LtG)` end-to-end.

**Built and verified today:**

- **Core seam & agents.** `unl-agent`: the `Agent` trait
  (`on_init`/`on_seed`/`on_message`/`on_tick`) + `Ctx`, and the wasm ABI
  (`init`/`run`/`config`/`deliver`/`alloc` exports, `send-unl` import). The
  `AgentRuntime`/Engine/Transport/StateStore/Clock/Crypto **adapter seams** (M1).
  The **N-agent executor** — one node hosts many agents on an in-process work
  queue — plus `SledStore`. Identity: `AgentId` (mint at spawn / persist for
  infra), `Aliases`.
- **The (from, unl, body) envelope**, `obj(verb, subject)` UNL, the DF/AMS/PA/BS
  verbs, and the full book-buy flow — verified on loopback (`book-cluster`) and in
  Docker over container IPs.
- **Secured cross-node transport.** R1: authenticated `from` — every `NodeMsg` is
  signed by the sending node's ed25519 key over `to`/`from`/`from_addr`/`unl`/
  `body`/`nonce`/`sender_pub` and verified on receipt; reserved-sender ids
  (`ams`/`df`/`pa`/`llm`/`boot`/…) are rejected inbound from the wire. R2: **Noise
  XX** encrypted, mutually-authenticated transport (per-node X25519 static key)
  over persistent per-peer connections. R4: wire hardening (`MAX_FRAME` cap before
  alloc; connect/read/write timeouts). Plus the original TCP routing
  (`process::node`): return-address caching, bootstrap routes, `RESOLVE` to the AMS
  node, startup registration.
- **Directory security.** R3: node-level **TOFU from-authorization** (first node
  key seen for a uuid owns it; impersonation rejected; a legitimate key change
  requires a signed handoff); AMS bind requires `from == agent`. R5: directory
  **quotas** (DF services/providers caps, AMS bindings cap — programmable). R6:
  **anti-fork** (AMS bindings epoch-monotonic; global location arbiter). R7:
  **metering** (per-call wasm fuel + memory limits; thread-per-connection serve;
  supervisor quarantines faulting agents).
- **Manifest & profiles (M2).** Manifest (HEAD) + `Capability`/`Profile`/`Brain`/
  `Budget`; `NodeProfile` (normal / iot) + **load-time fit**; the capability gate.
- **Scheduling (M3).** `Ctx::set_timer`/`cancel` + `Agent::on_tick`; gated timer
  slots.
- **Capabilities.** M4: **state** (namespace-confined `SledStore` handle + byte
  quota; R8 keys cannot escape the namespace); out-gate **net-scope**
  (`net="none"` sandboxes to local); the **async request-table**. M5: **crypto**
  (`sign`/`verify`/`random`, node-held key, domain-separated, confused-deputy
  defence) and **llm** (`infer` → off-thread `LlmBackend` → reply-by-message from
  `"llm"` by request_id). M6: **spawn** (child wasm, caps ⊆ parent),
  **supervisor** (fault quarantine), **audit** (`AuditSink`; log-rich node-side,
  uniform-deny agent-side).
- **Three brains, one seam.** `NativeRuntime` + `WasmRuntime` built; the Engine
  seam carries **two wasm backends** — wasmtime (JIT) and wasmi (IoT interpreter) —
  selected by node profile in `mount_wasm`. `LlmRuntime` is realized as the **llm
  capability** (the node runs the model).
- **Migration** (wasm only; native agents are stationary, host-instantiated
  templates): state-based snapshot/restore; signed `AgentSnapshot`; single-hop key
  handoff updating the AMS-node TOFU; epoch arbiter; crash-safety (tombstone only
  after destination ack); `CODE_FETCH` (content-addressed wasm by SHA-256,
  fetch-on-miss).
- **Platform agents:** AMS, DF, PA (durable sled ledger), plus BS (seller) and a
  real **wasm** Buyer (BA).
- **UNL stack:** `unl-core`, `unl-parser`, `unl-validator`, `unl-kb`, `unl-llm`,
  `unl-fipa`, `unl-a2a`.

**Designed but not yet built:**

- The **browser node** — node-as-wasm via `wasm-bindgen` (a separate project).
- **FIPA interaction protocols** in `unl-fipa` (request / query / contract-net /
  auctions / subscribe) — designed only.
- The **FIPA ACL envelope** (performative / conversation-id / reply-by) — not coded;
  discovery works today via direct DF/AMS messaging.
- The **full two-phase migration STAGING** (only the single-round-trip confirmed
  migration is built) and the **multi-hop attestation chain** (only single-hop
  handoff is built).
- **Per-time rate limiting** and the **AMS referral-loop bound**.
- The libp2p / Raft / Actix / WASI-P2 platform of [Appendix A](#appendix-a-future-directions-not-built)
  — still aspirational.

---

## 13. Glossary

- **AID** — FIPA Agent Identifier `{ name, addresses, resolvers }`; here `name` is a UUID.
- **AMS** — Agent Management System; white pages (UUID → address).
- **DF** — Directory Facilitator; yellow pages (service → providers).
- **PA** — Payment Agent; escrow.
- **Brain** — an agent's logic block (wasm | native | llm). *Not* the node.
- **Node** — the Rust resource manager / reference monitor that runs and gates agents.
- **Manifest / HEAD** — the agent's resource-management record, read first.
- **UNL** — Universal Networking Language; semantic graph content.
- **Tenant agent** — a bundle-loaded, gated agent. **Platform agent** — a native, trusted system agent (AMS/DF/PA).

---

## Appendix A: Future directions (not built)

An earlier draft of this document specified a richer distributed platform. None of
the following is implemented; it is recorded as a candidate roadmap, **not** a
description of the system:

- **Networking** — libp2p (mDNS + Kademlia discovery, NAT traversal, Noise/Yamux)
  in place of, or beneath, the current hand-rolled TCP transport; gRPC/tonic for
  typed RPC.
- **Consensus** — openraft for a replicated agent directory and service registry
  (linearizable `find`/`register`), with RocksDB-backed logs.
- **Supervision** — an Actix-style supervisor tree with per-agent restart strategies
  (immediate / backoff / max-failures / none).
- **Mobility** — agent migration and cloning via signed state snapshots
  (`doMove`/`doClone`), capturing linear memory, globals, conversation state, and
  storage.
- **Components** — WASI Preview 2 / the WebAssembly Component Model with WIT-typed
  interfaces, in place of the current minimal `(ptr,len)` ABI.
- **Federation** — multi-platform DF federation; MCP bridge so an external LLM can
  drive the platform (see `PLAN_MCP_AGENT.md`).

These should be adopted only deliberately, one at a time, measured against the
governing principles in §2 — especially P5 (an OS/transport-agnostic ABI) and the
small-agent/gating-node split.

---

## References

- [FIPA Specifications](http://www.fipa.org/repository/)
- [FIPA ACL](http://www.fipa.org/specs/fipa00061/) · [Agent Management](http://www.fipa.org/specs/fipa00023/)
- [UNL](http://www.unlweb.net/)
- [WebAssembly Component Model](https://github.com/WebAssembly/component-model)
- Companion: [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md), [`agents/AMS_DESIGN.md`](./agents/AMS_DESIGN.md), [`agents/DF_DESIGN.md`](./agents/DF_DESIGN.md), [`agents/PA_DESIGN.md`](./agents/PA_DESIGN.md)
