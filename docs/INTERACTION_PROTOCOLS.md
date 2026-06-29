# FIPA Interaction Protocols — implementation spec

**Version:** 0.2.0 (implementation-spec)
**Last Updated:** 2026-06-29
**Status:** spec complete; **not yet implemented**. Target crate: `unl-fipa`.
**Parents:** [`PROTOCOLS.md`](./PROTOCOLS.md) · [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md) · [`MOBILITY.md`](./MOBILITY.md)

Multi-message conversations (request, query, contract-net, iterated contract-net,
subscribe, English/Dutch/sealed-bid auctions) as concrete state machines over the
`(from, unl, body)` envelope, with the ACL header, content schemas, the `unl-fipa`
runtime, and worked messages. Every open question from v0.1 is resolved in §1.

---

## 1. Resolved decisions (was open in v0.1)

| # | Question | Resolution |
|---|---|---|
| 1 | ACL envelope encoding | A reserved **`_acl`** object at the top of the JSON `body`. Keys prefixed `_` are envelope; all other keys are domain content. **No wire/ABI change.** (§3) |
| 2 | Content schemas | Fixed per-performative `body` schemas (§5). |
| 3 | `conversation_id` | A UUIDv4 minted by the initiator at start, in `_acl.cid` (§3). |
| 4 | reply-by ↔ timer slots | Conversations do **not** each take a slot. `unl-fipa` runs **one** "protocol clock" timer slot + a deadline min-heap; N conversations multiplex over it (§6). |
| 5 | Multi-recipient send | No broadcast primitive. `cfp` fan-out is a **library loop of `send`**, shared `cid`; participant list comes from `find_service` (§6, §7). |
| 6 | Iterated-CN bounds / subscribe leases | `_acl.round` + `max_rounds`; subscriptions are **leased** (`_acl.lease_ms`) and need `renew` (§8, §9). |
| 7 | Auction gaps | Tie-break = earliest `propose` by receipt order; **sealed-bid = single-round CN** with a first-price or second-price (Vickrey) evaluator (§10). |
| 8 | Composition / nesting | `_acl.parent_cid` links a sub-conversation to its parent (§11). |
| 9 | Wire interop | Core wire stays **UNL+JSON**. FIPA-string/SL interop is an **optional gateway agent** using the mapping table (§12); not in the core. |

---

## 2. Where protocols live

Protocols are an **agent-side library** (`unl-fipa`), not a node subsystem (keeps the
trusted base thin; mirrors JADE `Behaviour`s). The node supplies only the primitives:
the ACL header (carried transparently in `body`), `send`, **one timer slot**, and
message delivery. Each protocol is an **initiator FSM** + a **responder FSM** the
agent drives via `send` + `tick`, correlated by `cid`.

---

## 3. The ACL header (`_acl`)

Carried as a reserved object at the top level of the JSON `body`. The `unl-fipa`
runtime adds it on send and strips it on receive; domain content are the sibling
keys. Platform agents (DF/AMS/PA) never set `_acl`, so they are unaffected.

```jsonc
"_acl": {
  "cid":  "<uuidv4>",        // conversation id (required)
  "pid":  "fipa-contract-net", // protocol id (required)
  "perf": "cfp",             // performative — the canonical intent (required, §4)
  "rw":   "<msg-id>",        // reply-with: id the sender wants echoed (optional)
  "irt":  "<msg-id>",        // in-reply-to: echoes the rw being answered (optional)
  "rb_ms": 5000,             // reply-by, RELATIVE ms from receipt (optional; skew-free)
  "round": 1,                // iterated-CN round (optional)
  "lease_ms": 60000,         // subscribe lease (optional)
  "parent_cid": "<uuidv4>",  // nesting link (optional, §11)
  "ont":  "books"            // ontology of the domain fields (optional)
}
```

Design choices: `rb_ms` is **relative** (milliseconds from receipt), so node clock
skew never affects a deadline — the receiver arms a local timer for `rb_ms`. The
canonical intent is `_acl.perf`; the `unl` verb mirrors it for human readability but
`perf` is authoritative.

---

## 4. Performatives and their UNL mirror

| `_acl.perf` | `unl` verb | Content (domain keys in `body`) |
|---|---|---|
| `request` | `obj(request, <subj>)` | `action` (UNL/JSON spec) |
| `agree` | `obj(agree, <subj>)` | — |
| `refuse` | `obj(refuse, <subj>)` | `reason` |
| `failure` | `obj(failure, <subj>)` | `reason` |
| `inform` (done) | `obj(inform, <subj>)` | — |
| `inform` (result) | `obj(inform, <subj>)` | `result` |
| `query-if`/`query-ref` | `obj(query, <subj>)` | `expr` |
| `cfp` | `obj(cfp, <subj>)` | `task` |
| `propose` | `obj(propose, <subj>)` | `bid` |
| `accept-proposal` | `obj(accept, <subj>)` | — |
| `reject-proposal` | `obj(reject, <subj>)` | — |
| `subscribe` | `obj(subscribe, <subj>)` | `pattern` |
| `inform` (update) | `obj(inform, <subj>)` | `update` |
| `cancel` | `obj(cancel, <subj>)` | — |
| `renew` | `obj(renew, <subj>)` | — |
| `not-understood` | `obj(nu, <subj>)` | `reason` |

`<subj>` is the task/topic/order concept word (e.g. `LtG`).

---

## 5. Content schemas

All `body` payloads are `{ "_acl": {…}, <domain> }`. The domain parts:

```jsonc
// cfp
{ "task": { "title": "Limits to Growth", "qty": 1, "deadline_ms": 5000 } }
// propose (a bid)
{ "bid": { "price": 999, "terms": { "ships_in_days": 3 } } }
// inform (result)
{ "result": { "order": "LtG-7f3a", "status": "ok" } }
// subscribe
{ "pattern": { "service": "bookselling", "match": "new-provider" } }
// inform (update)
{ "update": { "provider": "<uuid>", "service": "bookselling" } }
// refuse / failure / not-understood
{ "reason": "out-of-stock" }
```

`bid`, `task`, `pattern`, `result`, `update` are domain-defined objects; the protocol
FSM treats them opaquely except where it must (e.g. a CN evaluator reads `bid.price`).

---

## 6. The `unl-fipa` runtime

```rust
// One per agent. Routes inbound by cid, multiplexes deadlines over ONE timer slot.
struct Conversations {
    table: HashMap<Cid, Box<dyn Fsm>>,      // live conversations
    heap:  BinaryHeap<Reverse<(DeadlineMs, Cid)>>, // soonest deadline first
    slot:  TimerId,                          // the single protocol-clock slot
}

trait Fsm {
    // advance on an inbound protocol message; returns sends + a lifecycle step
    fn on_message(&mut self, acl: &Acl, body: &Json, out: &mut Vec<Send>) -> Step;
    fn on_timeout(&mut self, out: &mut Vec<Send>) -> Step;        // rb_ms fired
    fn deadline(&self) -> Option<DeadlineMs>;                     // next wake, if any
}
enum Step { Continue, Done(Value), Failed(Reason) }
```

Dispatch:
- **inbound**: parse `_acl`; route to `table[cid]` (or, for a responder's first
  message, instantiate the responder FSM for `pid`); step it; emit `out`; if
  `Done/Failed`, surface to the agent and drop the cid; recompute the heap top and
  re-arm `slot` to `now + (top.deadline - now)`.
- **tick(slot)**: pop every expired `(deadline, cid)`; call `on_timeout`; re-arm slot
  to the new top.

So **N conversations cost one timer slot.** A protocol-using agent declares
`budget.timers ≥ 1`. **Fan-out** (contract-net) is a plain loop emitting one `send`
per participant with the same `cid`; the participant list comes from a prior
`find_service` (ABI discovery).

---

## 7. fipa-request

```
I → R : request   {action}        _acl{cid, pid:"fipa-request", perf:"request", rb_ms}
R → I : agree | refuse{reason}
R → I : inform{} | inform{result} | failure{reason}
```

Initiator FSM states: `Sent —agree→ Agreed —inform→ Done(result?)`; `Sent —refuse→
Failed`; `Agreed —failure→ Failed`; any `tick` before the expected reply →
`Failed(timeout)`. Responder: `Recv —decide→ agree|refuse`, then on success
`inform`, on error `failure`.

---

## 8. fipa-query / fipa-contract-net / iterated

**query** (= request shape with `query`/`inform`):
```
I → R : query {expr}     R → I : inform {result} | refuse | failure
```

**contract-net** — one initiator, N participants:
```
I → P_i : cfp {task}           (loop; same cid; rb_ms = collection window)
P → I  : propose {bid} | refuse{reason}
   ── on tick(rb_ms) OR all-in: evaluate ──
I → win : accept                 I → others : reject
win → I : inform{result} | failure{reason}    (re-award on failure)
```
Initiator state: `Cfp{sent:N} → Collecting{props} —deadline/all→ Evaluated →
Awarded{win} —inform→ Done`; `—failure→ re-award or Failed`. The **evaluator is an
agent-supplied closure** `fn(&[Proposal]) -> Decision{ winner, losers }`; the FSM
calls it at the deadline. Late proposals (after `Evaluated`) get `reject`.

**iterated-contract-net** — re-issue refined `cfp` instead of awarding:
```
round k: cfp{task, _acl.round:k} → propose
  evaluate → satisfied ? accept/reject (terminate)
                       : cfp{revised, round:k+1} to the best subset
bounded by max_rounds (default 3) and per-round rb_ms
```

---

## 9. fipa-subscribe (leased)

```
I → R : subscribe {pattern}  _acl{cid, pid:"fipa-subscribe", lease_ms}
R → I : agree | refuse{reason}
R → I : inform {update}      (repeatedly, on each match)
... before lease_ms elapses ...
I → R : renew                 (re-arms the lease; R replies agree)
I → R : cancel                R → I : inform{} (closed)
```
The responder holds `Subscription{ subscriber, pattern, expires_at }`; it drops the
subscription on `cancel`, on lease expiry without `renew`, or on a send failure to
the subscriber. The initiator arms a `renew` timer at `lease_ms * 0.8`.

---

## 10. Auctions

Auctioneer `A` (initiator) ↔ bidders `B*`. `cfp`/`propose` loops; `pid:"fipa-auction"`
with `_acl` carrying `auction: "english"|"dutch"|"sealed"`.

**English (ascending):**
```
A → B* : cfp {price:p}      B → A : propose {bid:{price:p}}
A: ≥1 propose → p += step, re-cfp;   0 propose → accept(last bidder@last p), reject rest
```
**Dutch (descending):**
```
A → B* : cfp {price:p (high)}      A: no propose → p -= step, re-cfp
B → A : propose {price:p}          A: FIRST propose wins → accept(it), reject rest
```
**Sealed-bid = single-round contract-net:** one `cfp`, private `propose`s (each bidder
sends only to A — already the case), then A awards with:
- **first-price**: winner = max bid, pays own bid;
- **second-price (Vickrey)**: winner = max bid, pays second-highest.

**Tie-break (all auctions): earliest `propose` by receipt order.** Per-round `rb_ms`
bounds each announcement. Termination: English on a quiet round; Dutch on first
`propose`; sealed on the single deadline.

---

## 11. Composition (nesting)

A protocol may spawn a sub-conversation: the child `_acl.parent_cid` = the parent
`cid`. Example — a contract-net **award** triggers the escrow purchase:

```
… contract-net Awarded(winner) …
I starts fipa-request to winner:  request{action:"sell LtG"}  _acl{cid: c2, parent_cid: c1}
  (or proceeds to the PA escrow flow, PROTOCOLS.md §7)
```
The runtime keeps parent and child as independent FSMs; the agent links them by
`parent_cid`. No special node support.

---

## 12. Errors, timeouts, cancellation, interop

- **not-understood** — a responder that can't parse `perf`/content replies
  `obj(nu, <subj>)` `{reason}`; the initiator fails that conversation.
- **timeouts** — every wait is `rb_ms` → the protocol-clock slot → `on_timeout`.
- **cancel** — an initiator may `cancel` mid-flight; the responder unwinds
  (reservations, subscriptions) and replies `inform{}`.
- **FIPA interop (optional, out of core)** — a **gateway agent** translates between
  our `_acl`/UNL+JSON and FIPA ACL string/XML + SL, using this mapping:

  | ours (`_acl.perf`) | FIPA ACL |
  |---|---|
  | request/agree/refuse/failure/inform | request/agree/refuse/failure/inform |
  | query/cfp/propose/accept/reject | query-ref/cfp/propose/accept-proposal/reject-proposal |
  | subscribe/cancel/not-understood | subscribe/cancel/not-understood |

  Absolute FIPA `reply-by` ↔ our relative `rb_ms` (gateway converts using its clock).

---

## 13. Worked example — book-buy via contract-net

```
1. BA → DF  : obj(seek, bookselling)                → providers [s1, s2, s3]
2. BA → s_i : obj(cfp, LtG)  {_acl{cid:C,pid:"fipa-contract-net",perf:"cfp",rb_ms:4000},
                              task:{title:"Limits to Growth",qty:1}}      (i=1..3)
3. s1 → BA  : obj(propose, LtG) {_acl{cid:C,perf:"propose",irt…}, bid:{price:999}}
   s2 → BA  : obj(propose, LtG) {…, bid:{price:1050}}
   s3 → BA  : obj(refuse,  LtG) {…, reason:"out-of-stock"}
4. (tick rb_ms) evaluate → winner s1
   BA → s1 : obj(accept, LtG) {_acl{cid:C,perf:"accept-proposal"}}
   BA → s2 : obj(reject, LtG) {…}
5. BA → PA : obj(reserve, LtG) {seller:s1, amount:999}   … escrow (PROTOCOLS.md §7) …
6. ✓ obj(bought, LtG)
```
The escrow leg is the existing PA state machine; contract-net only replaces "take the
first provider" with competitive selection.

---

## 14. Status

| Piece | Status |
|---|---|
| `_acl` header + content schemas | ✅ specified |
| `unl-fipa` runtime (single-slot multiplex) | ✅ specified |
| request / query / contract-net / iterated-CN | ✅ specified |
| subscribe (leased) / auctions (eng/dutch/sealed) | ✅ specified |
| composition, errors, interop mapping | ✅ specified |
| **code** in `unl-fipa` | ⬜ post-M4 (needs ACL in `body` + scheduling M3 + async correlation M4) |

Prereqs: scheduling/`tick` (M3), async correlation (M4). No node changes beyond
carrying `_acl` transparently in `body` (already supported — it's just JSON).
