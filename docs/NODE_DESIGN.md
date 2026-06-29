# Node Design — the resource manager that backs the ABI

**Version:** 0.1.0 (draft)
**Last Updated:** 2026-06-29
**Status:** **largely implemented; see §15.**
**Parents:** [`ARCHITECTURE.md`](./ARCHITECTURE.md) · [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md)

`ARCHITECTURE.md` states *what* the system is; `AGENT_HOST_ABI.md` specifies the
*contract* between agent and host. This document describes *how the host (node) is
built* to honour that contract — the kernel, the adapters, the platform, the
lifecycles, and the build order.

---

## Table of Contents

1. [Organizing idea](#1-organizing-idea)
2. [The assembled node](#2-the-assembled-node)
3. [Kernel subsystems](#3-kernel-subsystems)
4. [The capability layer](#4-the-capability-layer)
5. [Adapter seams](#5-adapter-seams)
6. [Composition root and the profile matrix](#6-composition-root-and-the-profile-matrix)
7. [Core types](#7-core-types)
8. [Lifecycles](#8-lifecycles)
9. [Concurrency model](#9-concurrency-model)
10. [Security invariants](#10-security-invariants)
11. [IoT node](#11-iot-node)
12. [Browser node](#12-browser-node)
13. [Module layout](#13-module-layout)
14. [Implementation milestones](#14-implementation-milestones)
15. [Open decisions](#15-open-decisions)

---

## 1. Organizing idea

> **A portable Rust kernel + a set of swappable platform adapters.**

The **kernel** is identical on every target: it owns the gates, the manifest, the
capability layer, the scheduler, the supervisor, and the audit. Each target (normal
/ IoT / browser) supplies different **adapters** behind a handful of traits — engine,
transport, store, clock, crypto, llm — plus a **budget profile**. IoT and browser are
therefore **not forks**; they are adapter choices + a reduced capability set. This is
how the project keeps *one* ABI (and *one* agent code path) across three very
different deployment shapes, and it is the concrete form of governing principle P5
(OS/transport-agnostic ABI).

---

## 2. The assembled node

```
                                  ┌─ TENANT AGENTS (untrusted, gated) ──────────────┐
                                  │   BA(wasm)   BS(wasm)   …   each: brain+mailbox   │
                                  └───────▲───────────────────────┬──────────────────┘
                                          │ deliver/tick          │ upcalls (send, infer, state…)
┌──────────────────────────── NODE KERNEL (Rust, trusted) ────────┼──────────────────────────────┐
│                                                                  ▼                               │
│  Mailbox(per-agent) ─► Executor ─► IN-GATE ─► BRAIN ─► OUT-GATE ─► Capability layer              │
│     ▲   ▲                                                              │   │   │                 │
│     │   │ replies              ┌──── Async runtime + Request table ◄───┘   │   │                 │
│     │   └──────────────────────┘  (infer · find_service · locate · verify · spawn)               │
│     │                                                                      │   │                 │
│     │  Scheduler(timer slots) ──► tick                Supervisor(restart/quarantine)             │
│     │                                                                      │   │                 │
│     └─────────────── PLATFORM AGENTS (trusted, native): AMS · DF · PA ◄────┘   │ backs discovery │
│                                                                                │                 │
│  Audit sink ◄── every gate/fault (rich, node-attributed)        Keystore · Policy · Loader       │
└──────────┬──────────────┬──────────────┬──────────────┬──────────────┬──────────────┬───────────┘
   Engine ◇        Transport ◇     StateStore ◇      Clock ◇       Crypto ◇      LlmBackend ◇
```

---

## 3. Kernel subsystems

| Subsystem | Responsibility | ABI surface served |
|---|---|---|
| **Loader** | bundle → parse `HEAD`, verify `SIG`, **profile fit**, `granted = grants ∩ profile ∩ policy`, provision budgets, select brain, instantiate, `init`/`seed` | ABI §4 load sequence |
| **Supervisor** | owns mounted agents; lifecycle; restart/quarantine on fault; enforces fuel/mem budgets | downcalls; sandbox tiers |
| **Mailbox / Executor** | per-agent inbound queue; drives downcalls; drains upcall effects; one single-threaded loop per agent, N agents multiplexed | the run loop |
| **In-gate** | inbound `(from,unl,body)`: authenticate `from`, rate-limit, authorize | gate IN |
| **Out-gate** | every `OutboundIntent`: classify message-vs-host-call, validate net-scope/rate/size/budget, dispatch | gate OUT |
| **Capability layer** | the gated upcall handlers (§4); grant + budget check; sync result or `request_id` | all upcalls |
| **Async runtime + request table** | issue `request_id`, track in-flight (+ deadline + cap), run async work, reply via in-gate | ABI §8 |
| **Scheduler + clock** | timer-slot table (budget `timers`), fire `tick(timer_id, now_ms)`; wall + monotonic clocks | `time`, `tick` |
| **State store** | agent-scoped, quota'd durable KV (namespace = agent UUID) | `state` |
| **Audit sink** | structured, node-attributed forensic log; rich on violation/fault | ABI §11 |
| **Platform** | node-init bring-up of AMS/DF/PA as native agents; **discovery host-calls route here** | `discovery` backing |
| **Keystore / identity** | node keypair, per-agent keypairs; `SIG` verify; cross-node `from` signing; the crypto signing oracle | `crypto`, SIG |
| **Policy** | node-local overrides on grants/budgets/net-scope | load-time fit |

This is today's `process::node::Node` generalized: `deliver`/`emit`/`address_of`/
`resolve_local`/`register`/`serve` become, respectively, the executor + in-gate, the
out-gate, the discovery/transport split, the platform backing, the registration step,
and the transport loop.

---

## 4. The capability layer

Node-side handling of every gated upcall (contract in ABI §7):

| Upcall | Node does | Sync/async | Profile notes |
|---|---|---|---|
| `send(to,unl,body)` | out-gate → resolve `to` → transport | fire-and-forget | all |
| `find_service(svc)` | rid → message local **DF** → `deliver(from="df",{rid,providers})` | async-shaped | iot: single best, local |
| `locate(id)` | rid → ask local **AMS** → `deliver(from="ams",{rid,address})` | async-shaped | iot: static table |
| `get/put/del(key)` | agent-scoped namespace; quota on `put` | **sync** | iot KB / browser OPFS |
| `now/mono` | clock adapter | **sync** | iot: coarse |
| `timer_set/cancel` | slot table, budget `timers` | **sync** | iot: few slots |
| `infer(prompt)` | rid → `LlmBackend.complete` → `deliver(from="llm",{rid,text})` | async | **iot: denied** |
| `sign/random` | keystore (node-held key) / CSPRNG | **sync** | browser: in-wasm ed25519 |
| `verify(id,bytes,sig)` | rid → fetch pubkey (via AMS) → `deliver(from="crypto",{rid,ok})` | async | iot: optional |
| `spawn(bundle_ref)` | rid → Loader, caps ⊆ parent + quota → `deliver(from="node",{rid,child})` | async | **iot: denied** |
| `log(level,msg)` | audit sink, node-stamped | **sync** | all |

Rules: sync upcalls are **local and instant only** (run inline in the host import and
return within the call); anything touching network, another agent, or a model is
**async** and replies by message; `discovery` is a typed façade over the platform
agents, never a bypass of them.

---

## 5. Adapter seams

Six traits make the kernel portable. Each has one impl per target; introducing them
up front (even with a single impl) *is* the portability story.

| Trait | Role | normal | iot | browser |
|---|---|---|---|---|
| `Engine` | instantiate + run a wasm brain | wasmtime (JIT) | wasmi (interp) | browser WASM (JS glue) |
| `Transport` | the ACC/MTS wire | TCP | TCP → MQTT → BLE → LoRa | WS / WebRTC / BroadcastChannel |
| `StateStore` | agent-scoped durable KV | sled | in-RAM + flash | OPFS sync handles |
| `Clock` | wall + monotonic + timer source | OS | timer-IRQ / coarse | setTimeout |
| `Crypto` | sign / verify / random (key node-held) | native ed25519 | ed25519 / secure-element | in-wasm ed25519 |
| `LlmBackend` | `unl_llm::ReasoningBackend` | ollama / HTTP | — (denied) | fetch / WebGPU |

`WasmRuntime` becomes generic over `Engine`; `NativeRuntime` and the new `LlmRuntime`
keep the `AgentRuntime` seam, so the kernel stays brain-agnostic.

---

## 6. Composition root and the profile matrix

A `NodeBuilder` selects profile + adapters, mounts the platform, then mounts tenant
agents. The kernel code is identical across columns; `bin/mesh_node.rs`'s env config
+ `build_agent` becomes `NodeBuilder::from_env().profile(…).build()`.

| | normal | iot | browser |
|---|---|---|---|
| platform agents | AMS·DF·PA(+) | AMS·DF (minimal) | AMS·DF or remote gateway |
| budgets | MB / megafuel | KB / kilofuel | tab limits |
| capabilities | full | no `llm`, no `spawn` | full-ish (no FS) |
| async runtime | tokio | cooperative poll loop | cooperative poll loop |

---

## 7. Core types (design-level)

- **`Node`** — `agents: Map<Uuid, MountedAgent>`, the adapters, `platform`,
  `requests: RequestTable`, `scheduler`, `audit`, `policy`, `keystore`.
- **`MountedAgent`** — `id`, `manifest`, `granted: CapSet`, `budget: Budgets`,
  `runtime: Box<dyn AgentRuntime>`, `mailbox: Queue<Inbound>`, `timers: SlotTable`,
  `state_ns`.
- **`Manifest`** — extends `identity::Header` with `profile`, `brain`, `blocks`,
  `grants`, `budget`.
- **`Inbound`** — `(from, unl, body)` + optional `request_id` (async replies).
- **`RequestEntry`** — `rid → {agent, capability, deadline}`.
- **adapter traits** — `Engine`, `Transport`, `StateStore`, `Clock`, `Crypto`,
  `LlmBackend`.

---

## 8. Lifecycles

**(a) Boot** — load config + node keypair → pick profile/adapters → mount platform
agents (native) → wire routes → open transport → register → ready.

**(b) Mount a tenant agent** — Loader: parse HEAD → verify SIG → **profile fit**
(reject operator-facing if grants exceed profile) → compute `granted` → provision
budgets / state-ns / timer-slots → instantiate brain via `Engine` → `init()` →
`seed(unl,data)`.

**(c) Inbound round-trip** — the hot path:
```
transport ─► IN-GATE (auth from, rate, authorize) ─► mailbox.push
executor pulls one ─► runtime.deliver(from,unl,body)
   ├─ sync upcalls run INLINE: state/time/log/sign/random → result returned in-call
   └─ async upcalls issue request_id INLINE, schedule work, return id (non-blocking)
runtime returns ─► drain effects ─► OUT-GATE each (scope/rate/size) ─► messaging→transport
```
One message per agent at a time → **serialized per agent**, no in-agent races.

**(d) Async round-trip** (`infer`, `find_service`, …):
```
agent: rid = infer(prompt)                       // returns instantly
node:  RequestTable[rid] = {agent, llm, deadline}
       async: LlmBackend.complete(prompt)        // off the agent thread
       (discovery → message local DF/AMS, continuation keyed by rid)
done:  reply ─► IN-GATE ─► mailbox.push( deliver(from="llm",{rid,result}) )
agent: deliver() matches rid → continues
```
Gate failure → opaque `denied`; operational outcome (no provider, model down) →
informative reply (ABI §10).

**(e) Timer fire** — Scheduler fires slot → `mailbox.push(tick(timer_id,now_ms))` →
executor → `runtime.tick(...)` → drain → out-gate. The agent multiplexes its own
behaviours over slots.

**(f) Fault / shutdown** — panic (native `catch_unwind`) / wasm trap / budget breach
→ output discarded → Supervisor quarantines or restarts per policy, full audit
record. `shutdown()` releases holds, deregisters from AMS/DF, flushes state.

---

## 9. Concurrency model

- **Per agent**: a mailbox + single-consumer loop → serialized, single-threaded
  (sound for wasm). N agents run independently.
- **Node-wide async runtime**: capability work (LLM HTTP, transport I/O, discovery)
  runs off the agent thread; replies re-enter via the in-gate. *normal*: tokio.
  *IoT / browser*: a cooperative poll loop with non-blocking adapters — exactly why
  the adapter seams exist.
- **Sync upcalls never block** — they are local host imports returning inline,
  enforced by the "sync ⇒ sync-capable primitive" constraint (§15 / P5).

---

## 10. Security invariants

The spine, assembled — each must always hold:

1. Every byte in passes the **in-gate**; every effect out passes the **out-gate**.
   No other path exists.
2. `from` is node-stamped, never agent-set.
3. An agent's authority = `granted` only; an ungranted call → opaque `denied`
   (gate failure), distinct from an informative operational result of a granted call.
4. Capability work — LLM, **crypto keys**, cost, model — executes **node-side**; the
   agent holds only request/reply. Crypto keys never enter the sandbox; the node is a
   signing oracle with **domain separation** (ABI §7.2).
5. Audit is node-attributed and agent-unspoofable; **log rich, reply thin**.
6. A fault is contained to its agent; the node and other agents survive.

---

## 11. IoT node

| Area | Specialization |
|---|---|
| Engine | small wasm **interpreter** (wasmi-class); no JIT; low-RAM, deterministic |
| ABI set | messaging · discovery(local) · state(KB) · time(few slots) · log · crypto(optional). **No `llm`, no `spawn`** (rejected at load) |
| Platform | minimal AMS + DF local (possibly merged); PA optional → self-contained offline mini-platform |
| Transport | adapter trait; roadmap **TCP → MQTT → BLE → LoRa**; small frames; optional store-and-forward for intermittent links |
| State | in-RAM + optional flash; quota in **KB**; may be non-durable |
| Discovery | single best match, local DF, **static route table**, no dynamic RESOLVE — works offline |
| Scheduler | few slots, coarse resolution, **tickless low-power**: sleep between events, wake on timer or inbound |
| Budgets | KB memory, kilofuel, in-flight async cap 1–2 |
| Brain | wasm (mobility) or native; a mobile wasm agent built once runs here if its manifest fits |
| Crypto | key node-held; ed25519 native or secure-element; `random` from hardware entropy |

---

## 12. Browser node

| Area | Specialization |
|---|---|
| Where it runs | the Rust node **compiled to wasm**, hosted in a **Web Worker** (gives OPFS-sync + off-main-thread compute) |
| Engine for agents | the browser's WebAssembly engine; node-wasm instantiates agent-wasm via JS glue; agent imports are JS shims calling back into node-wasm (nested wasm) |
| Transport | no raw TCP → **WebSocket** (gateway), **WebRTC** (peer), **BroadcastChannel/postMessage** (tab-local) |
| State | **OPFS sync access handles** (honours the sync ABI) or in-memory + async write-back |
| Crypto | **in-wasm ed25519** (sync); avoid async WebCrypto; key still node(-wasm)-held |
| LLM | `infer` async → `fetch()` to an endpoint, or **WebGPU** local model; `LLM` block = a URL |
| Scheduler | `setTimeout`/`setInterval` drive timer slots → `tick` |
| Platform | AMS/DF in-page, **or** delegate to a remote platform over WS (browser node as a mesh leaf) |
| `spawn` | = instantiate another agent-wasm in the page (gated, quota'd) — not OS processes |
| Use case | offline/edge UIs; an agent runs in the user's browser; a **mobile wasm agent migrates into a web page** |

---

## 13. The sync-on-async-host constraint

"Sync upcalls must be local + instant" (ABI §7) forces **sync-capable primitives** on
every target — this is P5 with teeth:

- `state` sync ⇒ **no IndexedDB** (async). Browser uses **OPFS sync access handles**
  (in a Worker) or in-memory + async write-back. IoT uses in-RAM + flash.
- `crypto.sign` sync ⇒ **no WebCrypto** (async). Use **pure-Rust ed25519 in the
  node-wasm**. (Key custody unaffected — the key is node-side regardless; ABI §7.2.)

Recording this prevents a browser/IoT port from discovering, late, that it cannot
honour the sync ABI.

---

## 14. Module layout

Where the design lands in the crate, and where existing code moves:

```
fipa-wasm-agents/src/
  node/
    mod.rs  builder.rs       # NodeBuilder composition root   (← generalizes bin/mesh_node.rs)
    kernel.rs                # Node, executor, mailbox         (← today's process/node.rs)
    loader.rs  manifest.rs   # mount + HEAD                    (← extends identity::Header)
    gate.rs                  # in-gate + out-gate              (← node.rs deliver/emit, hardened)
    caps/{messaging,discovery,state,time,llm,crypto,spawn,log}.rs
    async_rt.rs  scheduler.rs  supervisor.rs  audit.rs  policy.rs
    platform.rs              # AMS/DF/PA bring-up + discovery backing (← node.rs register/resolve_local)
  adapters/
    engine.rs  transport.rs  store.rs  clock.rs  crypto.rs  llmbackend.rs
  wasm/  runtime.rs (WasmRuntime, generic over Engine) + llm_runtime.rs (new LlmRuntime)
```

`WasmRuntime` / `NativeRuntime` keep the `AgentRuntime` seam; `LlmRuntime` joins them.

---

## 15. Implementation milestones

Sequenced so each step is independently testable and the book-buy keeps passing
throughout:

Status legend: **DONE** = on git `main`, tested, `book-cluster` green · **PARTIAL** =
core built, remainder noted · **PENDING** = not yet built.

| M | Deliverable | Proves | Status |
|---|---|---|---|
| **M1** | Adapter traits + refactor current node onto `Engine=wasmtime`/`Transport=tcp`/`Store=sled`; N-agent mailbox/executor | seams exist; `book-cluster` still green | **DONE** |
| **M2** | `Manifest` + `Loader` (parse/fit/grant/budget) + `Policy`; gates made explicit | load-time fit; uniform denials | **DONE** |
| **M3** | `Scheduler` + `time` caps + `tick` | autonomy (timeouts, retries) | **DONE** |
| **M4** | `async_rt` + `RequestTable`; `discovery` & `state` caps; out-gate scoping (+ per-namespace quota) | async-reply model end-to-end | **DONE** — discovery works via direct DF/AMS messaging; typed discovery host-calls not separately built |
| **M5** | `LlmBackend` + `LlmRuntime` + `infer`; `crypto` (node-held key, domain-separated) | LLM brain + tool; signing oracle | **DONE** — incl. migration (state-snapshot/restore, signed `AgentSnapshot`, single-hop handoff, epoch arbiter, ack-before-tombstone crash-safety, content-addressed `CODE_FETCH`); full two-phase STAGING + multi-hop attestation chain **PARTIAL** |
| **M6** | `Supervisor` + `Audit` subsystem; hard fuel/mem metering; spawn caps ⊆ parent | containment + forensics | **DONE** |
| **M7** | IoT profile (wasmi, reduced ABI, local platform, low-power, MQTT) | second shape on one kernel | **PARTIAL** — wasmi interpreter backend + profile-based engine selection in `mount_wasm` **DONE**; constrained transports (MQTT/BLE/LoRa) and low-power scheduling **PENDING** (TCP only) |
| **M8** | Browser profile (node-as-wasm, OPFS, WS/WebRTC) | third shape; mobility into a page | **PENDING** (separate wasm-bindgen project) |
| **E1** | `Engine`/`EngineModule` seam + wasmtime backend (`WasmRuntime` through the seam) | engine portability | **DONE** |
| **E2** | wasmi backend, profile-selected; persistent Noise connections | constrained engine + durable links | **DONE** |

---

## 16. Open decisions

1. ~~**Async runtime** — tokio for normal + a cooperative-loop trait for IoT/browser,
   or a single runtime-agnostic executor from day one?~~ **RESOLVED:** tokio is **not**
   used for the node loop — it is a single-threaded poll loop + thread-per-connection.
2. ~~**Wasm effects** — direct gated host imports (out-gate sees each `send`/`infer`
   inline) vs. the `Ctx`/`take_sends` collect-then-drain model?~~ **RESOLVED:** wasm uses
   direct gated host imports (`send-unl`) over the five-op `Engine` seam; both still hit
   the out-gate.
3. ~~**Manifest format** — JSON `HEAD` now, compact binary as an IoT optimization
   later?~~ **RESOLVED:** `Manifest` is JSON `HEAD`.
4. ~~**First code** — start at **M1** (a pure refactor, no behaviour change).~~
   **RESOLVED:** started at M1; M1–M6 now done.
5. ~~**Agents per kernel** — one agent or N?~~ **RESOLVED:** the kernel hosts **N** agents.
