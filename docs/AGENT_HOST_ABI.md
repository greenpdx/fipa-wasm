# Agent ↔ Host ABI

**Version:** 0.1.0 (draft spec)
**Last Updated:** 2026-06-29
**Status:** the capability ABI + gating + Engine seam are **implemented and tested** — see [§13](#13-mapping-to-code-reuse-vs-new).
**Parent:** [`ARCHITECTURE.md`](./ARCHITECTURE.md)

This document specifies the contract between an **agent** (a small, untrusted bundle
of blocks) and the **host node** (the trusted Rust resource manager). It is the
single seam across which all authority flows, and therefore the place where all
security is enforced.

---

## Table of Contents

1. [Trust boundary](#1-trust-boundary)
2. [Block layout](#2-block-layout)
3. [The manifest (HEAD)](#3-the-manifest-head)
4. [Load sequence](#4-load-sequence)
5. [Profiles and the capability matrix](#5-profiles-and-the-capability-matrix)
6. [Downcalls — host → agent](#6-downcalls--host--agent)
7. [Upcalls — agent → host (gated)](#7-upcalls--agent--host-gated)
8. [The async model](#8-the-async-model)
9. [Scheduling — generic timer slots](#9-scheduling--generic-timer-slots)
10. [Gating and the error model](#10-gating-and-the-error-model)
11. [Audit and logging asymmetry](#11-audit-and-logging-asymmetry)
12. [Wire conventions](#12-wire-conventions)
13. [Mapping to code (reuse vs new)](#13-mapping-to-code-reuse-vs-new)
14. [Open items](#14-open-items)

---

## 1. Trust boundary

```
                    ▲ untrusted (the agent: a bundle of blocks)
   ─────────────────┼──────────────────────────────────────────  THE ABI (this spec)
                    ▼ trusted   (the node: Rust reference monitor)
```

- The **node** is trusted, native Rust. It loads agents, runs their brains, and
  mediates every call. It cannot be a wasm agent — it is the sandbox.
- The **agent** is untrusted. It has **no ambient authority**: its only powers are
  the downcalls the node makes into it and the upcalls the node chooses to honour.
- Everything the agent receives passes the **in-gate**; everything it emits passes
  the **out-gate**. There is no other channel.

**Cross-node trust (see [`THREAT_MODEL.md`](./THREAT_MODEL.md)).** The in-gate now
authenticates `from` **cross-node**, so a remote `from` is no longer forgeable. The
following requirements are **BUILT**:
- **R1 — BUILT.** Authenticated `from` cross-node: the sender node signs the envelope
  `(from,to,unl,body,nonce)` and the in-gate verifies it against the peer node identity
  (`MOBILITY §7`). Reserved sender-ids (`ams`/`df`/`pa`/`llm`/`node`/`crypto`/`boot`/
  `resolver`/`result`) are **rejected if inbound from the wire**. Each instance is
  bound to a spawn-minted instance UUID (AID).
- **R2 — BUILT.** Authenticated, encrypted transport: mutual node auth + Noise in the
  `Transport` adapter, with TOFU peer-key pinning (R3).
- **R4 — BUILT.** Wire-codec hardening: hard `MAX_FRAME` cap before allocation;
  read/accept/connect timeouts; thread-per-connection serve (a slow peer can't stall
  the loop).
- **R7/R8 — BUILT.** Fuel/memory metering; agent state keys confined to the agent's
  UUID namespace (no `"../"` escape).
The full catalogue, severities, and the worked kill chain are in `THREAT_MODEL.md`.

---

## 2. Block layout

An agent bundle is a set of named blocks (see [`ARCHITECTURE.md` §5](./ARCHITECTURE.md#5-agents-are-block-bundles)):

| Block | Required? | Meaning |
|---|---|---|
| `HEAD` | **yes** | the manifest — read first, always |
| `WASM` | one brain block required | agent code, run by the node's wasm engine |
| `UNL`  | optional | vocabulary / system frame, decoded by the node |
| `LLM`  | one brain block required | model bytes or a reference the node runs |
| `DATA` | optional | static seed, delivered via `seed()` |
| `STATE`| optional | mutable durable state, node-owned and agent-scoped |
| `SIG`  | recommended | signature over the bundle, verified before load |

Exactly one **brain** is selected by `HEAD.brain` (`wasm` | `native` | `llm`).

---

## 3. The manifest (HEAD)

The manifest is the agent's **resource-management record**. It extends today's
`identity::Header` (`type`, `desc`, `name`).

```jsonc
{
  // identity (exists today)
  "type": "<uuid>",            // agent TYPE id
  "desc": "book-selling service",
  "name": "bookSeller",        // friendly name, display only

  // resource management (new)
  "profile": "normal",         // "normal" | "iot" | "either" — which node shapes it fits
  "brain":   "wasm",           // "wasm" | "native" | "llm" — which block is the brain
  "blocks":  ["wasm", "unl", "state"],          // which blocks are present
  "grants":  ["messaging", "discovery", "state", "time"],   // capabilities REQUESTED
  "budget":  {
     "mem_kb":    4096,        // linear-memory ceiling
     "fuel":      1e8,         // CPU/fuel ceiling per scheduling quantum
     "state_kb":  256,         // durable-state quota
     "timers":    4,           // schedulable slot count (see §9)
     "msg_per_s": 50,          // outbound message rate
     "net":       "platform"   // "none" | "platform" | "node:<id>,…" | "any"
  }
}
```

Rules:

- **`grants`** is a request, not a grant. The node intersects it with what the
  profile and local policy allow. The agent receives authority for the intersection
  only.
- **`budget`** fields are ceilings the node enforces. Omitted fields take profile
  defaults (smaller on IoT).
- The manifest is **signed** as part of the bundle (`SIG`); the node verifies before
  trusting any field.

---

## 4. Load sequence

The manifest is read **before anything else is done with the agent**. This *is* the
agent's resource management.

```
LOAD(bundle)
 1. parse HEAD
 2. verify SIG over the bundle            → reject if invalid
 3. profile fit: every grant ∈ this node's profile capabilities?
                                          → reject (operator-facing) if not  [load-time fit, §10]
 4. granted = grants ∩ profile ∩ policy   → the agent's effective authority
 5. provision budgets: memory limit, fuel meter, state quota, timer slots, rate limiter
 6. select brain (HEAD.brain) → instantiate runtime (Wasm | Native | Llm)
 7. init()                                → agent setup, no data
 8. seed(unl, data)                       → vocabulary + static seed (if present)
 ── agent is live: deliver() and tick() may now fire ──
```

Step 3 is the only place a node may reject for *capability* reasons, and it is
**operator-facing** and trusted. After step 6 the agent is running and all denials
are **agent-facing and uniform** (§10).

---

## 5. Profiles and the capability matrix

Two node profiles exist. The **ABI surface (downcalls + upcall names) is the same on
both** — what differs is which upcalls a profile *honours*, plus engine and budgets.
A node MAY link a reduced ABI on IoT for footprint; because mismatches are caught at
**load** (step 3), this does not let a running agent probe the surface.

| Capability | `normal` | `iot` | Tier |
|---|---|---|---|
| `messaging` | ✓ | ✓ | core |
| `discovery` | federated, multi-result | local DF, single best | core |
| `log` | ✓ | ✓ | core |
| `state` | ✓ (MB) | ✓ (KB) | opt-in |
| `time` | ✓ (many slots) | ✓ (few slots) | opt-in |
| `crypto` | ✓ | optional | opt-in |
| `llm` | ✓ | ✗ | opt-in / heavy |
| `spawn` | ✓ (gated) | ✗ | heavy / gated |
| brain engine | JIT | interpreter | — |

An agent that wants to be maximally mobile declares `profile: "iot"` or `"either"`
and requests only IoT-available grants; it is then admissible on any node shape.

**Discovery keeps the async *shape* on every profile** — `find_service`/`locate`
always return a `request_id` and reply by message (§8), even where an IoT node
resolves the answer instantly and replies on the next tick. This preserves a single
agent code path across profiles; only the *quality* of the answer (federated vs
local, multi vs single) differs, never the ABI shape.

---

## 6. Downcalls — host → agent

The node drives these entry points. (`tick` and an explicit `shutdown` are new;
the rest exist today.)

| Downcall | Signature (logical) | Purpose |
|---|---|---|
| `init` | `init()` | setup, no data |
| `seed` | `seed(unl, data)` | vocabulary + static seed (once, at load) |
| `deliver` | `deliver(from, unl, body)` | one inbound message |
| `tick` | `tick(timer_id, now_ms)` | a scheduled slot fired (see §9) |
| `shutdown` | `shutdown()` | teardown: release holds, deregister, flush state |
| `alloc` | `alloc(len) -> ptr` | wasm only: reserve an inbound buffer |

Notes:

- `deliver` is also how **async upcall results arrive** (§8): a reply is delivered
  as a normal inbound message with `from` set to the capability name
  (`"llm"`/`"df"`/`"ams"`) and the originating `request_id` echoed in `body`.
- The seam already collapses `init`/`seed`/`deliver` into `AgentRuntime::config`
  for native agents; `tick`/`shutdown` extend it.

---

## 7. Upcalls — agent → host (gated)

Each upcall is a capability, honoured only if granted (§4). Calls return either a
local result (sync) or a `request_id` (async, §8). Every outbound effect passes the
out-gate.

| Interface | Calls | Sync/async | Tier | Out-gate enforces |
|---|---|---|---|---|
| **messaging** | `send(to, unl, body)` | async (fire-and-forget) | core | net-scope, rate, size |
| **discovery** | `find_service(svc) -> request_id`, `locate(id) -> request_id` | async | core | read-only; profile variant |
| **state** | `get(key) -> bytes`, `put(key, bytes)`, `del(key)` | **sync** | opt-in | agent-scoped namespace, quota |
| **time** | `now() -> ms`, `mono() -> ns`, `timer_set(delay_ms, timer_id)`, `timer_cancel(timer_id)` | **sync** | opt-in | slot budget (§9) |
| **llm** | `infer(prompt) -> request_id` | async | opt-in/heavy | cost budget; runs the LLM block |
| **crypto** | `sign(bytes) -> sig`, `verify(id, bytes, sig) -> request_id`, `random(n) -> bytes` | sync (`sign`,`random`) / async (`verify`) | opt-in | **key node-held**; domain-separated (§7.2) |
| **spawn** | `spawn(bundle_ref) -> request_id` | async | heavy/gated | quota; child caps ⊆ parent |
| **log** | `log(level, msg)` | sync | core | node-attributed, unspoofable |

Design rules:

- **Sync upcalls are local and instant only** (`state`, `now`/`mono`, `timer_*`,
  `log`, `sign`, `random`). They never block on I/O or another agent.
- **Anything that touches the network, another agent, or a model is async** and
  replies by message (§8). The agent stays purely reactive; the node thread never
  stalls.
- `discovery` is a *typed façade* over the node's AMS/DF platform agents — promotion
  to host-calls does not bypass or replace them.

### 7.1 Where LLM processing happens

LLM inference is always a **node service** — never the agent. It lives behind one
adapter (`LlmBackend`, an `unl_llm::ReasoningBackend`) and is reached two ways:

- **as a brain** — an `llm`-brained agent: the node runs inference *as* the agent's
  think step (`LlmRuntime` maps each `deliver` → prompt → inference → outbound);
- **as a capability** — a wasm/native agent calls `infer(prompt)` as a tool and gets
  the result back by message (§8).

Both go through the same adapter, which decides *where the model runs*:

| `LLM` block | backend | where compute runs |
|---|---|---|
| weights (bytes) | embedded / local | in-node (CPU/GPU) |
| `ollama:<model>` | `OllamaBackend` | local Ollama (edge, CPU-only) |
| URL / model-id | HTTP backend | remote hosted API |
| (browser) URL or WebGPU | fetch / WebGPU | remote, or in-page GPU |

The node always mediates: it checks the `llm` grant, meters the **cost budget**,
holds the model and keys (the agent never sees them), logs the call, and for UNL
agents **constrains and validates** the output (`unl-llm`: the model proposes, the
validator disposes). **IoT denies `llm` outright.**

### 7.2 Crypto: key custody and domain separation

`crypto` keeps the **private key node-side** — in the keystore, never in the agent's
sandbox. The agent holds only the *operations* (`sign`/`verify`/`random`), so an
agent compromise or memory-disclosure bug cannot exfiltrate a key. The node is a
**signing oracle** (the HSM / ssh-agent pattern): faster (native constant-time
ed25519, one vetted implementation the node can patch) and consistent with the node
already holding LLM keys and cost. In-wasm *agent* crypto is rejected precisely
because it would leak a secret into the sandbox.

This introduces a **confused-deputy** risk — an agent coaxing the oracle to sign
bytes that mean something elsewhere (a forged attestation, a payment authorization).
It is closed by a mandatory rule:

- **The node controls *what* is signed, not just *whether*.** It never blind-signs
  raw agent bytes with an identity key: for message authentication the *node* builds
  the envelope (from, to, nonce, timestamp) and signs that; the agent supplies only
  content fields.
- **Per-purpose keys + a domain-separation tag** — identity ≠ message ≠ app-data
  keys; the node prefixes a domain tag before signing.
- `sign` is gated, **rate-limited, and audited** like any host-call.

Notes:

- **`verify` carries no secret** — safe to host; the node also fetches/caches trusted
  public keys via AMS.
- **`random` *must* be a host call** — wasm has no entropy source, so an in-wasm PRNG
  is a real vulnerability (predictable keys/nonces). Host `random` supplies OS
  entropy. This is a correctness fix, not just performance.
- **Browser**: "host crypto" is the *node-wasm's* pure-Rust ed25519 (WebCrypto is
  async and cannot satisfy a sync `sign`). The key-custody guarantee still holds; the
  raw-speed win applies mainly to native profiles.

---

## 8. The async model

No upcall blocks. An async capability returns a `request_id` immediately; the node
performs the work and delivers the result back through the **normal inbound path**.

```
agent: rid = find_service("bookselling")        // returns immediately
 node: out-gate ─► route to local DF agent ─► collect providers
 node: deliver(from="df", unl=<result graph>, body={ "request_id": rid, "providers": [ … ] })
agent: deliver() handler matches rid → continues its logic
```

- **Correlation** is by `request_id`, echoed in the reply `body`.
- The same mechanism serves `infer`, `locate`, `verify`, `spawn`.
- An agent therefore needs no new "callback" ABI: an async result is just a message
  from a well-known capability id. This keeps the surface minimal and uniform.
- The node MAY cap in-flight async requests per agent (a budget); over the cap →
  `denied` (§10).

---

## 9. Scheduling — generic timer slots

The host provides a **dumb, generic** scheduling primitive. It knows nothing about
"behaviours"; the agent multiplexes its own units of work over slots.

- `timer_set(delay_ms, timer_id)` — arm slot `timer_id` to fire after `delay_ms`.
- `timer_cancel(timer_id)` — disarm it.
- The **slot budget** is `HEAD.budget.timers = N`. Allocation is either
  **static** (fixed ids `0..N`) or **dynamic** (the agent allocates ids up to `N`),
  chosen by the node.
- On fire, the node calls **`tick(timer_id, now_ms)`**; the agent dispatches the
  behaviour it associated with that id.
- Arming beyond budget → `denied`.

This gives JADE-style multiple independent schedulable units while keeping the host
primitive trivial and the policy (which behaviour, repeating or one-shot) entirely
inside the agent.

---

## 10. Gating and the error model

Two times, two audiences:

- **Load-time fit** (step 3) — trusted, **operator-facing**. The node may reject an
  agent whose grants exceed the profile, with a precise reason in the operator log.
  This is where IoT footprint reduction lives.
- **Runtime denial** — untrusted, **agent-facing**, **uniform**. Once admitted, any
  disallowed call — ungranted-by-manifest, absent-by-profile, over-budget,
  rate-limited, scope-violation — returns the **same opaque `denied`**. The agent
  cannot tell the reasons apart and cannot probe the host.

**Denial vs operational failure.** The uniform opaque `denied` covers only
*permission / capability / budget gate* failures — the anti-probing surface. The
*operational outcome* of a **granted** call is informative: `find_service` with no
provider, `infer` against an unreachable model, or `verify` returning false are the
legitimate answers to a call the agent was allowed to make, and are delivered as
normal (async) replies (§8). Gate failures are opaque; granted-call results are not.

Budgets enforced at runtime: memory, fuel/CPU, state quota, timer slots, outbound
message rate, in-flight async cap. Exceeding any → `denied` (for a call) or
quarantine (for memory/fuel) with a full node-side record (§11).

---

## 11. Audit and logging asymmetry

**Log rich, reply thin** — absolute (P6).

| Event | Audit channel (node-side, trusted, maximal) | Agent reply (untrusted, minimal) |
|---|---|---|
| permission violation | agent UUID+type, node id, capability, exact call + args (or hashes), the grant/profile/budget rule that denied it, manifest grants, timestamp, correlation id, sequence # | uniform `denied` |
| failure (panic/trap/quota) | brain kind, fuel/mem at fault, panic payload (native) or wasm trap, state snapshot ref | uniform terminal status; output discarded |

- The audit record is **node-attributed**: the `log` capability is node-stamped, so
  an agent can neither forge nor suppress its own violation record.
- A probing agent that walks every capability receives an undifferentiated wall of
  `denied`, while the audit log captures the entire walk — the strongest abuse
  signal the system has.

---

## 12. Wire conventions

The logical signatures above flatten to the existing minimal ABI style — no WASI, no
Component Model — so the contract is **OS/transport-agnostic** (P5) and a browser
node can implement it.

- **Strings/bytes** cross as `(ptr: *const u8, len: usize)` pairs. Example (today's
  `send-unl` import): `send(rp, rl, up, ul, bp, bl)` for `(to, unl, body)`.
- **Inbound buffers**: the node calls `alloc(len)` in the guest, writes the bytes,
  then calls the downcall (`deliver`/`tick`).
- **`request_id`** is an opaque node-issued token (string), echoed in async reply
  bodies.
- **Results** of sync upcalls are returned via a small `(ptr,len)` out-param region
  the node reads, or a status word — to be fixed when implemented.
- No filesystem, no sockets, no clock syscalls are exposed to the guest: `now`/`mono`
  are upcalls, state is an upcall, transport is the node's.

---

## 13. Mapping to code (reuse vs new)

**Reuse (already in tree, BUILT on):**

- `unl_agent::Agent` / `Ctx` / `Outgoing` and the `export_agent!` ABI
  (`init`/`run`/`config`/`deliver`/`alloc`, `send-unl` import). ✓
- `wasm::AgentRuntime` seam (`init`/`config`/`take_sends`/`run`/`shutdown`),
  `WasmRuntime`, `NativeRuntime` (fault isolation). ✓
- `wasm::host::OutboundIntent` — the out-gate's unit. ✓
- `identity::Header` — extended into the manifest. ✓
- `unl_llm::ReasoningBackend` / `Prompt` — the engine `LlmRuntime` drives. ✓

**Status — BUILT (on `main`, tested; the 5-node book-cluster verifies `obj(bought, LtG)`):**

1. ✅ Manifest/HEAD (`Capability`/`Profile`/`Brain`/`Budget`) + `SIG` + the **load
   sequence** (§4), **load-time profile-fit** and the capability gate (§5/§10).
2. ✅ Profiles (`normal`/`iot`) and budget provisioning.
3. ✅ **Scheduling**: `time` upcalls (`Ctx::set_timer`/`cancel`) + slot budget +
   `on_tick` downcall (§9).
4. ✅ The **out-gate** as an explicit stage: `send-unl` routes via host import to the
   node out-gate (scope/rate/size on every `OutboundIntent`).
5. ✅ The **async model** (§8): `request_id` issuance + reply-by-message (`deliver`)
   for `llm`/`crypto.verify`/`spawn`.
6. ✅ Upcall interfaces: `state` (`Ctx::state_get`/`put`/`del` → namespace-confined
   `SledStore` handle + byte quota), `llm` (`Ctx::infer` → off-thread `LlmBackend`,
   async reply from `"llm"` by `request_id`), `crypto` (`Ctx::sign`/`verify`/`random`,
   node-held key, domain-separated), `spawn` (child wasm, caps ⊆ parent).
7. ✅ `LlmRuntime` (`llm`-brained agents) + `LlmBackend` capability path.
8. ✅ Hard resource metering: the wasm execution seam is the `Engine`/`EngineModule`
   five-op interface (`refuel`/`call_void`/`call_i32`/`call_io`/`call_packed`) with
   **two backends** (wasmtime + wasmi/IoT), profile-selected; native fault boundary.
9. ✅ The **audit logging** subsystem (§11): log-rich node-side / thin-to-agent via
   `AuditSink`.

**Still NOT built (planned):**

- **Typed discovery host-calls** — discovery today works via direct DF/AMS
  *messaging*, not a separate discovery host-call surface (item 6 above intentionally
  omits it).
- **ACL envelope** — performative / conversation-id / reply-by fields.
- The **FIPA interaction protocols** (separate doc).
- A **browser** engine backend.
- **Full two-phase migration staging.**

---

## 14. Open items

Resolved (now BUILT):

- ~~**Sync upcall return convention**~~ — fixed by the `Engine`/`EngineModule` seam:
  sync results cross via `call_io`/`call_packed` (§12, §13).
- ~~**`spawn` lineage**~~ — `spawn` enforces "child caps ⊆ parent"; quota inheritance
  is in place.
- ~~**Cross-node `from` signing**~~ — the sender node signs the envelope and the
  in-gate authenticates a remote `from` (R1 + Noise transport R2 + TOFU R3); §1.

Still open:

- **`net` scope grammar** — the precise syntax for `HEAD.budget.net`.
- **Key custody on migration** — node-held keys do not travel with a mobile agent; a
  destination node needs a key-handoff, or per-node ephemeral keys plus attestation.
  Tied to full two-phase migration staging (not yet built).
- **State migration** — whether `STATE` travels with a mobile agent and how it is
  re-bound on the destination node (ties to the migration roadmap in
  `ARCHITECTURE.md` Appendix A).
