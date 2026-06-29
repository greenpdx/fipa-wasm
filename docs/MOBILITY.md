# Agent Mobility — migration and cloning — implementation spec

**Version:** 0.2.0 (implementation-spec)
**Last Updated:** 2026-06-29
**Status:** **implemented for wasm agents** — snapshot/restore, signed `AgentSnapshot`,
single-hop signed handoff, AMS epoch arbiter, crash-safety (ack-then-tombstone), and
content-addressed `CODE_FETCH` are built on `main` over a Noise-encrypted transport with
the node keystore. **Remaining (planned):** full two-phase STAGING (separate
PREPARE/PREPARED before COMMIT/COMMITTED), the multi-hop attestation chain + compaction
(only single-hop handoff is built), `SIG` wire-field naming, and browser-side migration.
**Parents:** [`ARCHITECTURE.md`](./ARCHITECTURE.md) · [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md) · [`NODE_DESIGN.md`](./NODE_DESIGN.md) · [`INTERACTION_PROTOCOLS.md`](./INTERACTION_PROTOCOLS.md)

**Weak, state-based** mobility: agents migrate at message boundaries (no raw
linear-memory capture), so migration is engine-portable — the same agent lands on a
normal, IoT, or browser node. This spec fixes the snapshot format, the two-phase
commit, code transfer, the attestation chain (the v0.1 open problem), and replay
protection. Every open question from v0.1 is resolved in §1.

---

## 1. Resolved decisions (was open in v0.1)

| # | Question | Resolution |
|---|---|---|
| 1 | Key handoff / attestation | **owner delegation cert + per-hop handoff chain**, each link Ed25519-signed; verifier walks owner→node₀→…→node_k (§7). **DONE (single-hop):** signed `Handoff` updates the destination/AMS-node TOFU key. *Multi-hop chain + compaction: planned.* |
| 2 | Transactionality | **two-phase move** (PREPARE/PREPARED/COMMIT/COMMITTED/ABORT) + epoch ⇒ exactly-once at a message boundary; crash cases enumerated (§6). **DONE (crash-safety):** single-round-trip confirmed migration — source tombstones only after `KIND_MIGRATE_ACK`. *Full two-phase STAGING (separate PREPARE/PREPARED before COMMIT): planned.* |
| 3 | Snapshot + code transfer | `AgentSnapshot` is JSON (CBOR on IoT); WASM is **content-addressed** (`wasm_hash`) and **fetched on miss** via a `CODE_FETCH` frame, not shipped inline (§4–5). **DONE:** content-addressed wasm by SHA-256, fetched on miss. |
| 4 | Conversation/timer capture | `unl-fipa` exposes `export()/import()` of `ConversationSnapshot`s; timers captured as **remaining-ms** (§4, §8). **DONE:** guest snapshot/restore via `export_agent!`. |
| 5 | Replay protection | destination keeps a persisted, TTL-bounded **seen-set** of `(uuid, epoch)`; epoch strictly increases (§9). **DONE:** AMS epoch arbiter (epoch-monotonic bind = anti-fork). |
| 6 | Clock skew | timers are **relative remaining-ms**, re-anchored to the destination clock; cert windows use a **±skew tolerance** (§8, §7). |
| 7 | Native agents | **native (big static) agents do not migrate** — only wasm/llm-brained agents do (§3). **DONE:** native/Rust agents are stationary, host-instantiated from templates; only wasm agents are mobile. |
| 8 | State store move | `StateStore::export(ns)/import(ns,bytes)`; state is already UUID-namespaced (§4). **DONE:** state-based snapshot/restore moves the namespaced durable state. |

---

## 2. Identity continuity

The **instance UUID (AID `name`) never changes on migration** — it is
location-independent (`identity.rs`). Migration is, at the directory level, an **AMS
re-bind** (`UUID → new address`) + a DF re-offer. Every durable reference (an escrow
hold naming the agent, an open conversation) stays valid because it names the UUID,
not the location. A **clone** is different: it is a new agent and **mints a fresh
UUID** (§3).

---

## 3. What moves

State-based, not memory-snapshot. **Only wasm- and llm-brained agents migrate;
native agents are stationary infrastructure** (AMS/DF/PA) and are not movable.

| Carried | Form |
|---|---|
| identity | `instance_uuid`, `type`, `owner_pubkey` |
| manifest | `HEAD` (incl. `wasm_hash`) |
| code | **reference** `wasm_hash` (bytes fetched on miss, §5) |
| state | `StateStore::export(ns)` blob |
| conversations | `unl-fipa` `ConversationSnapshot[]` |
| timers | remaining-ms per armed slot |
| provenance | `origin_node`, `epoch`, `nonce`, `history[]` |
| attestation | the delegation+handoff chain (§7) |

Raw linear memory + wasm globals are **not** carried (engine-specific, fragile, large
attack surface). Restore = re-instantiate the brain fresh + replay state +
conversations + timers. This is exactly what makes a wasmtime→wasmi→browser move
work. Strong mobility (mid-call stack capture) is a **non-goal** (§11).

---

## 4. AgentSnapshot

```jsonc
AgentSnapshot {
  "instance_uuid": "<uuid>",
  "type": "<uuid>",
  "owner_pubkey": "<ed25519-pub b64>",
  "head": { /* manifest, incl. "wasm_hash":"<sha256>" */ },
  "state": "<base64 of StateStore::export(ns)>",
  "conversations": [ { "cid","pid","role","fsm_state","vars","deadline_remaining_ms" } ],
  "timers": [ { "timer_id", "remaining_ms" } ],
  "epoch": 7,                       // monotonic; ++ only on a COMMITTED move (§6,§9)
  "nonce": "<random 16B b64>",
  "origin_node": "<node-pubkey b64>",
  "history": [ "<node-pubkey>", … ],
  "chain": [ /* delegation + handoffs, §7 */ ],
  "sig": "<ed25519 over canonical(all-above-except-sig) by origin_node key>"
}
```

`StateStore::export(ns) -> bytes` / `import(ns, bytes)` move the agent's
UUID-namespaced durable state. Conversation FSM state comes from
`unl-fipa::export()`. The snapshot is JSON on normal/browser, **CBOR on IoT** (size).

---

## 5. Code transfer (content-addressed, fetch-on-miss)

WASM is identified by `wasm_hash = sha256(wasm_bytes)`, recorded in `HEAD` and signed
by the owner in the bundle `SIG`. The snapshot carries the **hash, not the bytes**. On
restore:

```
dest has wasm_hash in its code store?  → use it (dedup across migrations)
                       otherwise       → CODE_FETCH(wasm_hash) frame to origin_node
                                          ← CODE_BLOB(wasm_hash, bytes); verify sha256
```

This keeps snapshots small and lets many agents/migrations share one cached module.
The dest **verifies the fetched bytes hash** before instantiating, so a wrong/forged
blob is rejected.

---

## 6. The two-phase move (exactly-once)

New wire frame kinds (alongside `PROTOCOLS.md` §3): `MIGRATE_PREPARE(snapshot)`,
`PREPARED(token)`, `MIGRATE_COMMIT(epoch)`, `COMMITTED`, `ABORT(reason)`,
`CODE_FETCH(hash)`, `CODE_BLOB(hash, bytes)`.

```
SOURCE (owns live agent A at epoch e-1)
 P1. quiesce A: stop new deliveries; finish in-flight downcall; checkpoint pending request_ids
 P2. build snapshot at epoch e (= e-1 + 1); sign; send MIGRATE_PREPARE → dest

DEST
 P3. verify sig + chain (§7); reject (uuid,e) if in seen-set (§9); profile-fit HEAD
     (a heavy agent cannot land on IoT — same gate as a fresh mount, ABI §4)
 P4. fetch code if needed (§5); instantiate; restore state/conversations/timers
     → agent staged, NOT resumed; reply PREPARED(token); start a staging timeout

SOURCE
 P5. on PREPARED → send MIGRATE_COMMIT(e)

DEST
 P6. resume A; AMS bind UUID→my addr; DF re-offer; record (uuid,e) COMMITTED in seen-set;
     reply COMMITTED

SOURCE
 P7. on COMMITTED → set e-1 := e; tombstone local A + run a short forwarder (§8);
     drop the forwarder after the grace window
```

**Crash/loss cases:**
- *source dies after PREPARE, before COMMIT* → dest's staging timeout fires →
  discards staged A. Source still holds the live A at e-1 → **no loss, no duplicate.**
- *COMMIT sent, COMMITTED ack lost* → source retries COMMIT(e); dest sees (uuid,e)
  already committed → **idempotent**, re-acks.
- *dest dies before COMMITTED* → source never advances; aborts and keeps A live;
  may retry with the same e (dest's stale staging timed out).

Epoch advances **only on a received COMMITTED**, so exactly-once holds at the message
boundary.

---

## 7. Attestation chain (key handoff)

Keys stay node-side at every hop (ABI §7.2). Identity is the **UUID**, not any signing
key; authority to sign *as the agent* is conveyed by a verifiable chain.

**Three keys:** the **owner** (the principal that deployed the agent; `owner_pubkey`
in `HEAD`, signs the bundle `SIG`), and a per-**node** key at each host.

**Two link types (Ed25519-signed):**

```jsonc
Delegation {            // owner authorizes the FIRST node to act for the agent
  "agent": "<uuid>", "to_node": "<node0-pub>",
  "nbf": <ms>, "naf": <ms>, "epoch": 0,
  "sig_by_owner": "<sig>"
}
Handoff {               // node_{i-1} hands the agent to node_i at migration i
  "agent": "<uuid>", "from_node": "<node_{i-1}-pub>", "to_node": "<node_i-pub>",
  "epoch": i, "snapshot_hash": "<sha256>",
  "nbf": <ms>, "naf": <ms>, "sig_by_from_node": "<sig>"
}
```

**Verification** of a message claimed from agent A, hosted at node N_k:
1. bundle: `SIG` valid by `owner_pubkey` over `(HEAD‖wasm_hash)` → owner authorized
   this code+identity.
2. `Delegation`: signed by owner, `to_node = N_0`, time-valid.
3. each `Handoff_i`: signed by `from_node = N_{i-1}`, `to_node = N_i`, `epoch = i`,
   strictly increasing, time-valid; `to_node` of link i = `from_node`/signer of i+1.
4. the live message: signed by `N_k` (the current host) — the chain ends authorizing
   `N_k`.

Only a node in the authorized chain can sign as A; custody never leaves the node side.
Time windows use a **±skew tolerance**. **Compaction:** the owner may issue a fresh
`Delegation` (higher epoch) directly to the current node, collapsing the chain.
**Revocation:** owner publishes a higher-epoch `Delegation` (counterparties prefer the
highest epoch) and/or a short CRL via DF; short `naf` windows bound exposure.

---

## 8. Consistency & forwarding

- **in-flight messages** — between P1 and P6 the source buffers/forwards: after P7 it
  runs a **forwarder** for a grace window (default 30 s) relaying stray messages to
  the new address; AMS update makes this transient.
- **durable refs** — escrow holds, DF entries, open conversations name the unchanged
  UUID → valid across the move.
- **pending async `request_id`s** — drained at quiesce (P1) if possible, else re-armed
  at the dest; a reply to a stale rid is dropped.
- **timers** — captured as **remaining-ms**, re-armed against the destination's
  monotonic clock, so `reply_by` keeps its meaning despite clock skew.

---

## 9. Replay protection

The destination keeps a **persisted seen-set** of `(instance_uuid, epoch)` with a TTL
of `2 × max_migration_time` (default 10 min). A `MIGRATE_PREPARE` whose `(uuid, epoch)`
is already COMMITTED is rejected. `epoch` is a monotonic counter in the agent's
persisted state, advanced only on COMMITTED (§6); handoff epochs must strictly
increase, preventing a rollback to an older node.

---

## 10. Security invariants

1. Mobility is a **heavy, gated, default-denied** capability (ABI §7); denied on IoT.
   *Enforced: only wasm agents are mobile; native agents are stationary.*
2. Snapshots are **signed** by the origin node and verified before a byte is trusted.
   *Enforced in code: `AgentSnapshot` carries an origin-node Ed25519 signature over
   `uuid/epoch/code_hash/state/nonce/origin_pub`; verified before mount. MIGRATE runs
   over a Noise-encrypted transport keyed from the node keystore.*
3. A migrated agent gets **no elevated trust** — profile-fit + grant-intersection run
   again at the destination, identically to a fresh mount.
4. Code is **hash-verified** on fetch (§5); chain + epoch give origin authenticity and
   replay resistance. *Enforced in code: `CODE_FETCH` is content-addressed and the
   fetched bytes are SHA-256-verified; the AMS epoch arbiter gives epoch-monotonic
   (anti-fork) binding. Single-hop signed handoff is enforced; the multi-hop chain is
   still planned.*
5. Every migration is **audit-logged** node-side (origin, dest, epoch, snapshot hash).

---

## 11. API, browser, non-goals

**API** (two triggers, same §6 sequence):
- agent-initiated: gated `migrate(node) -> request_id` / `clone(node) -> request_id`
  upcalls (heavy), replying by message (ABI §8). `clone` mints a fresh UUID and binds
  it; the original keeps running.
- platform-initiated: the supervisor migrates for load-balance / shutdown-drain; the
  agent receives a lifecycle notice.

**Browser node as destination:** verifies the snapshot, instantiates the agent-wasm
via the browser engine, imports state into OPFS, re-binds with the (possibly remote)
AMS over WebSocket. Possible **only because** migration is state-based (no
engine-specific memory image). A heavy agent still fails profile-fit (P3).

**Non-goals:** **strong mobility** (mid-call stack/linear-memory capture) — we do weak,
state-based mobility at message boundaries: engine-portable and far safer.

---

## 12. Status

**Implemented for wasm agents** (snapshot/handoff/epoch/crash-safety/CODE_FETCH);
full two-phase staging + multi-hop chain compaction remain.

| Piece | Status |
|---|---|
| `AgentSnapshot` + state/conversation export (`export_agent!`) | ✅ built |
| signed `AgentSnapshot` (origin-node Ed25519 sig) | ✅ built |
| content-addressed code transfer (`CODE_FETCH`, SHA-256-verified) | ✅ built |
| crash-safety (ack-then-tombstone via `KIND_MIGRATE_ACK`) | ✅ built |
| epoch arbiter (AMS epoch-monotonic bind = anti-fork) | ✅ built |
| single-hop signed handoff (TOFU key update) | ✅ built |
| node keystore + Noise-encrypted MIGRATE transport | ✅ built |
| full two-phase STAGING (PREPARE/PREPARED before COMMIT/COMMITTED + abort) | ⬜ planned |
| multi-hop attestation chain (delegation + handoff) + compaction | ⬜ planned |
| `SIG` wire-field naming | ⬜ planned |
| browser-side migration | ⬜ planned |
