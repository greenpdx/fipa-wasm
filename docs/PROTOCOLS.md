# Protocols — message envelope, wire format, and inter-agent conversations

**Version:** 0.1.0
**Last Updated:** 2026-06-29
**Status:** the verb tables below are **implemented and tested today**; the
envelope/async/interaction layers marked *planned* are not.
**Parents:** [`ARCHITECTURE.md`](./ARCHITECTURE.md) · [`AGENT_HOST_ABI.md`](./AGENT_HOST_ABI.md) · [`NODE_DESIGN.md`](./NODE_DESIGN.md)

Where `AGENT_HOST_ABI.md` specifies the *agent↔host* contract, this document
specifies the *agent↔agent* and *node↔node* protocols: the message envelope, the UNL
verb convention, the TCP wire format, and the conversation verb-tables for the
platform agents (DF, AMS, PA) and the book-buy flow.

---

## Table of Contents

1. [The message envelope](#1-the-message-envelope)
2. [UNL verb convention](#2-unl-verb-convention)
3. [Node↔node wire format](#3-nodenode-wire-format)
4. [Discovery — DF (yellow pages)](#4-discovery--df-yellow-pages)
5. [Discovery — AMS (white pages)](#5-discovery--ams-white-pages)
6. [Node registration & resolution](#6-node-registration--resolution)
7. [Escrow — PA](#7-escrow--pa)
8. [Catalog & fulfilment — BS](#8-catalog--fulfilment--bs)
9. [The book-buy conversation](#9-the-book-buy-conversation)
10. [Planned layers](#10-planned-layers)
11. [Status matrix](#11-status-matrix)

---

## 1. The message envelope

Every agent-level message is the triple **`(from, unl, body)`**:

| Field | Type | Carries | Rule |
|---|---|---|---|
| `from` | string (UUID) | the **authenticated** sender id | node-stamped, never agent-set |
| `unl` | UTF-8 UNL graph | **human/semantic** content — the verb + subject | UUIDs never appear here |
| `body` | bytes (JSON) | **structured machine data** — UUIDs, addresses, amounts | the machine half |

This is the project's core rule (P7): **UNL for meaning, JSON for structured data.**
A reply is sent to `ctx.from()`.

**Planned — the ACL envelope.** FIPA ACL fields (`performative`,
`conversation-id`, `in-reply-to`, `reply-with`, `reply-by`, `protocol`, `ontology`)
are an **opt-in** layer to ride alongside `(from, unl, body)` — they are *not*
implemented today (conversations currently correlate by subject/order id). See
[§10](#10-planned-layers).

---

## 2. UNL verb convention

Every platform message uses one UNL relation of the form:

```
obj(<verb>, <subject>)
```

parsed as: the first relation's **source word = `<verb>`**, its **target word =
`<subject>`** (relation label `obj`). Two subject styles:

- **placeholder `agent`** — when the real subject is a UUID (structured data), the
  UNL subject is the literal placeholder word `agent` and the UUID travels in the
  JSON body. Used by AMS (`obj(locate, agent)`), and node `bind`.
- **concept word** — when the subject *is* semantic (a service, a topic, an order
  id), it appears directly: `obj(seek, bookselling)`, `obj(reserve, LtG)`.

This keeps UNL purely semantic and machine ids in the body.

---

## 3. Node↔node wire format

Cross-node transport (`process::node`) is a length-prefixed TCP framing. A frame is:

```
[ kind : u8 ] [ len : u32 BE ] [ payload : len bytes ]

kind = 1  MSG          payload = an encoded NodeMsg
kind = 2  RESOLVE_REQ  payload = a UUID (ascii)
kind = 3  RESOLVE_RESP payload = an address (ascii; empty ⇒ unknown)
```

A `NodeMsg` is five length-prefixed (`u32 BE` + bytes) fields, in order:

```
to · from · from_addr · unl · body
```

`from_addr` is the sender's **return address**, cached by the receiver so replies
have a route without a lookup. One message is one connect-write-(read for
RESOLVE)-close. This is the FIPA *Agent Communication Channel* at the byte level;
it is the `Transport` adapter's normal-profile impl (browser/IoT swap the framing).

---

## 4. Discovery — DF (yellow pages)

Find providers by **what they do**. `<service>` is a UNL concept word (exact-match in
v1; embedding-ranked later). Provider registers itself (`from` = the provider).

| Action | in: `unl` / `body` | DF does | reply → `from`: `unl` / `body` |
|---|---|---|---|
| register | `obj(offer, <service>)` / — | add `<service> → from` (idempotent) | `obj(registered, <service>)` / — |
| search | `obj(seek, <service>)` / — | look up providers | `obj(provide, <service>)` / `["<id>", …]` |

An empty result is `[]`, not an error. Unknown verbs are ignored (no
`not-understood` in v1).

---

## 5. Discovery — AMS (white pages)

Resolve an **agent id → address**. The agent in question is a UUID, so it rides in
the body; the UNL subject is the placeholder `agent`. Each reply echoes `"agent"` for
correlation.

| Action | in: `unl` / `body` | AMS does | reply → `from`: `unl` / `body` |
|---|---|---|---|
| bind | `obj(bind, agent)` / `{"agent":"<uuid>","address":"<addr>"}` | store `uuid → addr` | `obj(bound, agent)` / — |
| locate | `obj(locate, agent)` / `{"agent":"<uuid>"}` | resolve | **found:** `obj(at, agent)` / `{"agent","address"}` · **referral:** `obj(refer, agent)` / `{"agent","ams"}` · **not found:** `obj(at, agent)` / `{"agent"}` (no `address`) |

Three FIPA resolution modes: *direct* (`at`) and *referral* (`refer`) are the agent's
job; *recursion* (chasing a referral chain) is a node/FIPA-layer concern, not the
agent's.

---

## 6. Node registration & resolution

A node performs two protocol actions on behalf of its agent.

**Startup registration** (`Node::register`):
- `bind` its UUID→address with AMS — `obj(bind, agent)` / `{"agent":<me>,"address":<my-addr>}`.
- if it offers a service, `offer` it to DF — `obj(offer, <service>)`.

**Address resolution** (`Node::address_of` → `resolve_local`): when a node must route
to a UUID it has neither bootstrapped nor cached, it sends a `RESOLVE_REQ` frame
(§3) to the AMS node, which answers by asking its local AMS agent `obj(locate,
agent)` (from `"resolver"`) and returns the `address` field in a `RESOLVE_RESP`. The
address is then cached.

---

## 7. Escrow — PA

A six-verb escrow state machine over a durable ledger. The order id is the UNL
subject; terms are in the body. `from` authorizes accept/deny.

```
                 reserve (buyer=from)
        insufficient ╱ ╲ ok
              deny ◀╱   ╲▶ HELD ── receipt "held" → buyer & seller
                          │
            accept(seller)╱ ╲ deny(buyer|seller)
      release→seller, PAID   release→buyer, CANCELLED
       receipt "paid" ×2     receipt "cancelled" ×2
```

| Action | in: `unl` / `body` | PA does | reply |
|---|---|---|---|
| reserve | `obj(reserve, <order>)` / `{"seller":"<id>","amount":<n>}` | buyer = `from`; hold funds | `obj(receipt, <order>)` / `{"status":"held","amount"[,"buyer"]}` → buyer **and** seller; or `obj(deny, <order>)` / `{"reason"}` |
| accept | `obj(accept, <order>)` / — | release to seller (requires `from`==seller) | `obj(receipt, <order>)` / `{"status":"paid","amount"}` ×2 |
| deny | `obj(deny, <order>)` / — | refund buyer (requires `from`∈{buyer,seller}) | `obj(receipt, <order>)` / `{"status":"cancelled","amount"}` ×2 |

`reason` values: `duplicate-order`, `bad-request`, `bad-amount`, `insufficient`,
`unauthorized`. The held-receipt to the seller also carries `"buyer"` so the seller
can fulfil. Security baked in today: authorization (`from` vs the hold's
buyer/seller), idempotency (duplicate `reserve` rejected), amount validation. Signed
messages/receipts are deferred (PA_DESIGN roadmap).

---

## 8. Catalog & fulfilment — BS

The seller answers catalog queries and mirrors PA's escrow.

| in: `unl` / `body` | BS does |
|---|---|
| `obj(catalog, <topic>)` / — | reply `obj(catalog, <topic>)` / `[{"title","price"}, …]` to `from` |
| `obj(receipt, <order>)` / `{"status":"held","buyer"}` | reserve the book; `obj(accept, <order>)` → `pa` |
| `obj(receipt, <order>)` / `{"status":"paid"}` | `obj(deliver, <order>)` → buyer (ship) |
| `obj(receipt, <order>)` / `{"status":"cancelled"}` | release the reservation |

BS addresses `pa` (an alias the node resolves) and the buyer (by id); it is
transport-agnostic.

---

## 9. The book-buy conversation

The end-to-end flow that exercises every protocol above (the BA buyer drives it):

```
BA → DF  : obj(seek, bookselling)            DF → BA : obj(provide, bookselling) ["<bs-uuid>"]
BA → AMS : obj(locate, agent) {bs-uuid}      AMS → BA: obj(at, agent) {bs-uuid, address}
BA → BS  : obj(catalog, systemdynamics)      BS → BA : obj(catalog, …) [{LtG,999},…]
BA → PA  : obj(reserve, LtG) {seller,amount} PA → BA : obj(receipt, LtG) {held}
                                             PA → BS : obj(receipt, LtG) {held, buyer=BA}
BS → PA  : obj(accept, LtG)                  PA → BA : obj(receipt, LtG) {paid}
                                             PA → BS : obj(receipt, LtG) {paid}
BS → BA  : obj(deliver, LtG)                 BA      : ✓ obj(bought, LtG) → result sink
```

Correlation today is by **subject/order id** (`LtG`), not a conversation-id.

---

## 10. Planned layers

Not implemented; specified here so the gap is explicit (also tracked in
`AGENT_HOST_ABI.md` §14 and `NODE_DESIGN.md` §16):

- **ACL envelope** — `performative` (FIPA's 22 communicative acts), `conversation-id`,
  `in-reply-to`, `reply-with`, `reply-by` (deadline), `protocol`, `ontology`,
  alongside `(from, unl, body)`. Replaces today's subject-id correlation with proper
  conversation threading.
- **Async-reply correlation** — the host-call `request_id` echoed in async replies
  (`deliver(from="df"|"ams"|"llm", {request_id, …})`), per ABI §8. Today discovery is
  done by directly messaging DF/AMS, not via a typed host-call.
- **`not-understood` / error performatives** — uniform handling of unknown verbs
  (today they are silently ignored).
- **Signed messages & receipts** — end-to-end authenticity using the node-held
  crypto keys with domain separation (ABI §7.2); enables cross-node `from`
  authentication.
- **FIPA interaction protocols** — formal state machines for request, query,
  **contract-net**, subscribe, and the auction protocols. The book-buy is an ad-hoc
  flow today; these would be reusable, named protocols.

---

## 11. Status matrix

| Protocol | Status |
|---|---|
| `(from, unl, body)` envelope | ✅ implemented |
| `obj(verb, subject)` UNL convention | ✅ implemented |
| Node↔node TCP wire (`NodeMsg`, RESOLVE) | ✅ implemented |
| DF register/search | ✅ implemented + tested |
| AMS bind/locate (direct + referral) | ✅ implemented + tested |
| Node registration & RESOLVE | ✅ implemented + tested |
| PA escrow (reserve/accept/deny + receipts) | ✅ implemented + tested |
| BS catalog & fulfilment | ✅ implemented + tested |
| Book-buy end-to-end | ✅ verified (loopback + Docker over IP) |
| ACL envelope (performative, conv-id, reply-by) | ⬜ planned |
| Async `request_id` correlation | ⬜ planned |
| `not-understood` / error acts | ⬜ planned |
| Signed messages & receipts | ⬜ planned |
| FIPA interaction protocols (contract-net, auction, subscribe) | ⬜ future |
