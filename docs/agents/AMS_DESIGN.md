# Agent Management System (AMS) — design

The AMS is the **white pages**: it resolves an **agent id → its address**
(physical location). It is the system's DNS. Where DF answers *"who sells
books?"* with an agent id (`bookSeller`), AMS answers *"where is `bookSeller`?"*
with an address.

When asked **"where is ABC?"**, the answer comes back in one of three ways — the
same trichotomy as DNS — but split across two layers:

| Your phrasing | DNS term | who | reply |
|---|---|---|---|
| "I have it right here" (data or cached) | authoritative / cached | **agent** | `at` + address |
| "check XYZ AMS" | iterative / referral | **agent** | `refer` + upstream AMS |
| "I'll check XYZ AMS and tell you" | recursive / proxy | **FIPA layer** | `at` + address (resolver chased the chain) |

> Key split: the **AMS agent** only ever answers directly or refers. The
> **recursion** — chasing referrals to a final address — lives in the FIPA layer
> (`process::resolve`), so the agent stays simple. *Which* mode a resolver uses
> (iterative vs recursive) is a **policy decision deferred for later**; for now
> the caller passes it explicitly.

## 1. Where AMS sits in the book flow

```
BA → DF  : seek bookselling      DF → BA  : provider bookSeller
BA → AMS : locate bookSeller     AMS → BA : at bookSeller {address}
(then BA → bookSeller directly)
```

## 2. Message model

`(from, unl, body)` — **UNL** carries the action + subject agent; **JSON body**
carries structured data (the address, the referral target).

## 3. Agent actions

| Action | in: `unl` / `body` | AMS does | reply → `from`: `unl` / `body` |
|---|---|---|---|
| **bind** | `obj(bind, <agent>)` / `{"address":"<addr>"}` | store `<agent> → addr` (authoritative) | `obj(bound, <agent>)` / — |
| **locate** | `obj(locate, <agent>)` / — | resolve | `obj(at, <agent>)` / `{"address":…}` **or** `obj(refer, <agent>)` / `{"ams":…}` |

Agent decision logic:
```
have record?       → at  {address}        # "I have it right here"
elif upstream?     → refer {ams: upstream} # "check XYZ AMS"
else               → at {}                 # not found
```

## 4. Recursion — in the FIPA layer (`process::resolve`)

```
resolve(amses, start, agent, recursive, max_hops) -> Found(addr) | Referral(ams) | NotFound
```
- asks AMS `start` to `locate <agent>`;
- on **`at`** → `Found(address)` (or `NotFound` if empty);
- on **`refer`** →
  - `recursive = false` → return `Referral(<ams>)` (iterative: caller re-asks);
  - `recursive = true`  → follow to `<ams>` and loop (**proxy**);
- bounded by `max_hops` (referral-loop guard).

So a chain `leaf → root` resolves `bookSeller` in one call when recursive, or
returns the `root` referral when iterative. The agents never recurse.

## 5. Data model

```
records  : Map<agent, address>   # authoritative bindings + answers cached via bind
upstream : Option<ams-id>        # parent AMS, for referral / recursion
```
Seeded from the `DATA` block (`on_seed`):
```json
{ "records": { "bookSeller": "127.0.0.1:9001" }, "upstream": "ams-root" }
```
`upstream` absent ⇒ an authoritative-only leaf. **TTL on cache** is a v2 hook.

## 6. Edge cases

| Case | Behavior |
|---|---|
| unknown agent, no upstream | agent replies `at {}`; resolver → `NotFound` |
| referral loop | resolver bounded by `max_hops` → `NotFound` |
| `bind` overwrites | last write wins (authoritative) |

## 7. Crate / module layout

```
agents/ams/                 # ams-agent (rlib): struct Ams { records, upstream }
crates/fipa-wasm-agents/
  src/process/resolve.rs    # the FIPA-layer resolver (iterative / recursive)
```
Registered as `native_agent("ams")`.

## 8. Tests (all passing)

- agent: bind→locate (direct), locate-unknown→refer (referral),
  no-upstream→not-found, seed from DATA.
- resolver: direct, iterative (returns referral), recursive (chases the chain),
  unknown→not-found.

## 9. Deferred

- **Mode-selection policy** — when a resolver should be iterative vs recursive
  (RD flag, AMS recursion-available, hierarchy depth). For now: explicit param.
- **Cache TTL / expiry**; conversation-id correlation (vs the current synchronous
  ask-reply resolver).
