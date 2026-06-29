# Threat Model & Security Requirements

**Version:** 0.1.0
**Last Updated:** 2026-06-29
**Status:** offensive audit of the design + current code. Findings are **open** unless
marked otherwise; mitigations are **binding requirements** on the milestones named.
**Parents:** [`ARCHITECTURE.md`](./ARCHITECTURE.md) · [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md) · [`NODE_DESIGN.md`](./NODE_DESIGN.md) · [`PROTOCOLS.md`](./PROTOCOLS.md) · [`INTERACTION_PROTOCOLS.md`](./INTERACTION_PROTOCOLS.md) · [`MOBILITY.md`](./MOBILITY.md)

This is a red-team analysis of the protocols and node transport — *attacking* the
system to find how it breaks. It was performed against the docs and the implemented
code (`process::node`, `agents/{ams,df,pa,bs}`, `identity`). Most of the platform is
unbuilt, so these are **design-stage** fixes: cheap now, expensive later.

---

## Table of Contents

1. [Attacker model](#1-attacker-model)
2. [Assets](#2-assets)
3. [The systemic finding](#3-the-systemic-finding)
4. [Findings — Critical](#4-findings--critical)
5. [Findings — High](#5-findings--high)
6. [Findings — Medium / trust](#6-findings--medium--trust)
7. [The kill chain (worked)](#7-the-kill-chain-worked)
8. [Mitigation requirements](#8-mitigation-requirements)
9. [Residual & accepted risks](#9-residual--accepted-risks)
10. [Security test plan (post-code)](#10-security-test-plan-post-code)
11. [Findings status matrix](#11-findings-status-matrix)

---

## 1. Attacker model

We assume an attacker who can:

- **reach a node's transport endpoint** — the normal case: Docker bridge, IoT link,
  LAN, or the internet. Today the transport is plaintext, unauthenticated TCP.
- **send arbitrary frames** — craft any `NodeMsg` (`to`, `from`, `from_addr`, `unl`,
  `body`) and any control frame.
- **run their own node** and participate in the platform (offer services, bind
  addresses, migrate agents).
- **deploy a malicious agent** within the rules (it is sandboxed, but may probe).

We assume the attacker **cannot** (yet) break Ed25519 or read node-private keystore
memory. Out of scope for v0.1: physical attacks, side-channels on the host OS.

**Trust boundaries.** The node (Rust) is the TCB for *its* agents. Across nodes there
is currently **no authenticated boundary** — that is the core problem (§3).

---

## 2. Assets

| Asset | Threat if lost |
|---|---|
| escrow funds (PA ledger) | theft, griefing, fund lockup |
| agent identity (`from`/AID) | impersonation → all authorization defeated |
| directory integrity (AMS/DF) | traffic hijack, service impersonation |
| node availability | DoS of a node / the platform |
| agent state (durable) | corruption, cross-agent disclosure |
| migration integrity | agent forking / double-spend |
| LLM keys / cost (node-held) | exfiltration, cost abuse |

---

## 3. The systemic finding

> **`from` is authenticated only *intra-node*. Cross-node it is attacker-controlled —
> and every authorization decision in the system trusts `from`.**

`Router` stamps `from` for in-process agents, but `process::node::deliver` takes
`from` directly off the wire and hands it to the agent. PA escrow auth, conversation
correlation, async-reply routing, and discovery all trust `from`. For the cross-node
deployment that is the project's entire purpose, **authorization is forgeable by
anyone who can open a socket.** This is the load-bearing wall, and it is absent. Most
Critical/High findings are instances or consequences of this.

The docs list "cross-node `from` signing" as *planned*; this audit reclassifies it as
**blocking for any networked build**.

---

## 4. Findings — Critical

### C1 — Payment theft via service impersonation + forged `from`
**Component:** DF, AMS, PA, transport. **Status:** open.
Full chain (works against current code — see §7):
1. `obj(offer, bookselling)` to DF — DF authorizes no one (`df`: `registry…insert(from)`).
2. `obj(bind, agent){agent, address}` to AMS — AMS authorizes no one (`ams`:
   `records.insert(agent, addr)`).
3. Buyer discovers the attacker, reserves escrow naming `seller=<attacker>`.
4. Attacker `obj(accept, <order>)` → PA releases funds → no delivery. **Funds stolen.**

### C2 — White-pages poisoning (AMS `bind` unauthorized)
**Component:** AMS. **Status:** open.
`obj(bind, agent){agent:<victim>, address:<attacker>}` rebinds *any* UUID to the
attacker's address → full MITM of the victim's inbound traffic. No check that the
binder owns the UUID.

### C3 — Route-cache poisoning (return-address trust)
**Component:** `process::node::deliver`. **Status:** open.
`routes.insert(msg.from, msg.from_addr)` from unauthenticated wire fields. One message
with `from=<victim>, from_addr=<attacker>` redirects the victim's replies to the
attacker — per-node MITM, no AMS needed.

### C4 — Unauthenticated remote memory-exhaustion
**Component:** `process::node::read_frame`. **Status:** open.
`let mut p = vec![0u8; n]` with `n: u32` and **no max-frame cap**. One frame with
`len = 0xFFFFFFFF` → 4 GB allocation → OOM. One packet kills a node.

### C5 — Reserved system senders spoofable from the wire
**Component:** transport / in-gate. **Status:** open.
Agents/nodes trust `from ∈ {ams, df, pa, llm, node, crypto, boot, resolver, result}`.
Nothing rejects these as *inbound from the network*. Attacker injects `from="ams"
obj(at,agent){address:<evil>}`, `from="llm" {request_id,result}`, or `from="boot"
obj(start,buy)` → forged discovery/tool results and remote kickoffs. Also defeats the
async-reply model (ABI §8).

---

## 5. Findings — High

### H1 — Agent forking / double-spend on migration
**Component:** MOBILITY. **Status:** open (design).
The replay seen-set is **per-destination**; nothing globally prevents the same
`(uuid, epoch)` snapshot being committed at two nodes → two live copies → duplicated
escrow holds. Epoch monotonicity is enforced locally, trusting the source. Needs a
**global location arbiter** (authenticated AMS `bind` as the single commit point).

### H2 — No transport authentication or encryption
**Component:** transport. **Status:** open.
Plaintext TCP, no peer identity, no TLS/Noise. Any connecting process is a trusted
peer; all traffic is eavesdroppable and injectable. The node keypair is unused at the
transport layer.

### H3 — Self-inflicted DoS (single-threaded `serve`, blocking I/O)
**Component:** `process::node::serve`, `address_of`. **Status:** open.
One thread; per-connection blocking `read_exact` (2 s) ⇒ slow-loris serially stalls
the node. `address_of` does a **synchronous blocking connect to AMS during message
handling** ⇒ a slow/hostile AMS freezes the handler. No fuel metering ⇒ a looping
wasm agent hangs the executor.

### H4 — Resource-exhaustion flooding (no quotas / TTL / GC)
**Component:** DF, PA, subscribe. **Status:** open.
DF `offer`, PA `reserve` (unique order ids), subscriptions all grow memory/sled
unbounded. PA holds never expire ⇒ griefer locks buyer funds forever, or squats order
id `"LtG"` (`duplicate-order`) to block the real order.

---

## 6. Findings — Medium / trust

| ID | Finding | Component |
|---|---|---|
| M1 | Migration extends trust to **every node in the chain**; a malicious past host could have altered agent state before signing the snapshot. The chain proves *authority to host*, not *state integrity*. | MOBILITY |
| M2 | **Vickrey/sealed-bid trusts the auctioneer** (no proof of the second price). Trustless needs commit-reveal. | INTERACTION |
| M3 | **Attestation-chain length DoS** — verifiers walk all links; compaction is optional. Bound it. | MOBILITY |
| M4 | **Unbounded `rb_ms`/`lease_ms`** — sender-set deadlines aren't clamped → resource holding. | INTERACTION |
| M5 | **Referral loops** — no hop bound on AMS referral chasing → resolution-loop DoS. | AMS |
| M6 | **Integer overflow** — PA `credit`/`accept`/`deny` use `+=`, not `checked_add` (docs claim "overflow-checked"; code does not). | PA |
| M7 | **State key-namespace escape** — spec must forbid keys escaping the agent's UUID namespace (e.g. `"../other"`). | ABI/state |

---

## 7. The kill chain (worked)

```
Attacker A, buyer B, PA escrow, on a multi-node deployment (plaintext TCP).

1. A → DF  : NodeMsg{ from:A, unl:"obj(offer, bookselling)" }        # DF: no auth → A is a "bookseller"
2. A → AMS : NodeMsg{ from:A, unl:"obj(bind, agent)",                # AMS: no auth → A reachable
                      body:{agent:A, address:A_addr} }
3. B (honest): seek bookselling → [A] ; locate A → A_addr ; catalog ; chooses A
4. B → PA  : obj(reserve, ord){ seller:A, amount:999 }               # B's own funds held
5. A → PA  : NodeMsg{ from:A, unl:"obj(accept, ord)" }               # PA: from==hold.seller (A) → release
6. PA → A  : receipt paid (funds → A) ;  A ships nothing.            # THEFT complete
```
No forged `from` is even required for this variant — DF/AMS accept *legitimate*
self-registration with no authorization, which is enough. Forged `from` (C5) makes it
strictly worse (impersonate B to cancel, replay kickoffs, spoof results).

---

## 8. Mitigation requirements

Binding requirements, mapped to milestones (`NODE_DESIGN.md §15`). **R1–R4 block any
networked (multi-node) build.**

| Req | Requirement | Closes | Milestone |
|---|---|---|---|
| **R1** | **Authenticated `from` cross-node.** Sender node signs `(from,to,unl,body,nonce)` with its node key; in-gate verifies against the peer node identity + the agent's attestation chain (`MOBILITY §7`). Promote from "planned" to blocking. | C1,C5,F-spoof | **M1-blocking** |
| **R2** | **Authenticated, encrypted transport.** Mutual node auth + Noise/TLS in the `Transport` adapter; bind node identity to the connection. Reserved sender-ids rejected if inbound from the wire. | H2,C3,C5 | M1 |
| **R3** | **Authorize the directories.** AMS `bind` requires `from == agent` (or owner-signed binding); DF `offer` requires `offerer == from`, rate+quota limited. | C1,C2 | M2 |
| **R4** | **Harden the wire codec.** Hard `MAX_FRAME` cap; reject oversized `len` before allocating; read/accept/connect timeouts; non-blocking `serve` + async `address_of`. | C4,H3 | M1 |
| **R5** | **Bound every resource.** Quotas + TTL/GC on DF entries, PA holds (expiry + auto-refund), subscriptions; clamp `rb_ms`/`lease_ms`; bound referral hops and attestation-chain length; `checked_add` in PA. | H4,M3,M4,M5,M6 | M2,M4 |
| **R6** | **Global migration commit point.** Authenticated AMS `bind` (R3) is the single arbiter of an agent's current location ⇒ a snapshot commits at exactly one destination. | H1 | M5 |
| **R7** | **Fuel/memory metering + per-conn limits** so one agent/peer cannot hang or exhaust the node. | H3 | M6 |
| **R8** | **State namespace confinement** — keys cannot escape the agent's UUID namespace. | M7 | M4 |

---

## 9. Residual & accepted risks

- **Hosting node is the TCB for its agents.** A migrated agent is only as trustworthy
  as the least-trustworthy node in its chain (M1). Mitigation is operational (only
  migrate among trusted nodes) + audit; cryptographic state-history attestation is a
  future option.
- **Sealed-bid auctions trust the auctioneer** (M2) unless commit-reveal is added —
  documented as a known limitation of the simple form.
- **Owner-key compromise = agent-identity compromise** (inherent). Bounded by short
  delegation windows + revocation; revocation distribution must itself be
  authenticated (depends on R3).

---

## 10. Security test plan (post-code)

After implementation, this audit is re-run as **live exploitation**, not just review:

- **Wire fuzzing** — malformed/oversized frames, partial frames, slow-loris against
  `serve` (expects R4).
- **The C1 kill chain, scripted** — automated offer/bind/reserve/accept theft attempt
  (expects R1+R3 to block).
- **`from`-spoofing harness** — inject every reserved sender-id and forged `from` from
  a rogue node (expects R1+R2).
- **Directory abuse** — bind-poisoning, service-hijack, registry/hold/subscription
  floods (expects R3+R5).
- **Migration attacks** — replay a snapshot to two destinations (fork/double-spend;
  expects R6), tampered snapshots, chain-length bombs.
- **Resource limits** — looping agent, memory-bomb agent, deadline floods (expects
  R5+R7).

Each finding above becomes a regression test: the exploit must fail.

---

## 11. Findings status matrix

| ID | Severity | Status | Closed by |
|---|---|---|---|
| C1 payment theft | Critical | open | R1, R3 |
| C2 AMS bind poisoning | Critical | open | R3 |
| C3 route-cache poisoning | Critical | open | R1, R2 |
| C4 frame memory-exhaustion | Critical | open | R4 |
| C5 reserved-sender spoof | Critical | open | R1, R2 |
| H1 migration fork/double-spend | High | open | R6 |
| H2 no transport auth/encryption | High | open | R2 |
| H3 single-thread/blocking DoS | High | open | R4, R7 |
| H4 flooding / no quotas | High | open | R5 |
| M1 migration chain trust | Medium | accepted/operational | — |
| M2 auctioneer trust | Medium | accepted (documented) | commit-reveal (future) |
| M3 chain-length DoS | Medium | open | R5 |
| M4 unbounded deadlines | Medium | open | R5 |
| M5 referral loops | Medium | open | R5 |
| M6 PA overflow | Medium | open | R5 |
| M7 state namespace escape | Medium | open | R8 |

**Bottom line:** the capability/gating, uniform-denial, and key-custody designs are
sound. The **cross-node trust model is currently open** — the headline feature
(agents across nodes/IoT/browser) has no authenticated transport and a forgeable
`from`, and the directory + escrow protocols authorize on that field. R1–R4 must land
before any networked code.
