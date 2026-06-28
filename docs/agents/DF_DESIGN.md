# Directory Facilitator (DF) — design

The DF is the **yellow pages**: agents find each other by *what they do*
(service), not by name. A provider **registers** the services it offers; a
requester **searches** for providers of a service. DF answers with matching
provider id(s); the requester then resolves a provider's address via **AMS**.

> Scope: this is the per-agent action spec to build against, simple first but
> with the expansion hooks (semantic match, federation) drawn now so growth
> needs no rework. DF lives in its own crate `agents/df` (native, rlib).

## 1. Where DF sits in the book flow

```
BA → DF : search "sell-books"          DF → BA : provider "bookSeller"
(then BA → AMS to resolve bookSeller's address)
```

## 2. Message model

Every message is `(from, unl, body)`:
- `from` — the sender's id (`ctx.from()`).
- `unl` — a **UNL graph**: the action verb + the service. This is the semantic
  part, so DF can match services by meaning (embedding) later.
- `body` — optional **JSON** for the structured part (result lists, metadata).

UNL for *what it means*, JSON for *structured data*. v1 scope: **register +
search only**.

## 3. Actions

`<service>` is a UNL concept UW (e.g. `book-selling`) — a single UW in v1
(exact match), a richer UNL graph later (semantic/embedding match).

| Action | in: `unl` / `body` | DF does | reply → `from`: `unl` / `body` |
|---|---|---|---|
| **register** | `obj(offer, <service>)` / — | add `<service> → from` (idempotent; provider = sender) | `obj(registered, <service>)` / — |
| **search** | `obj(seek, <service>)` / — | look up providers of `<service>` | `obj(provide, <service>)` / `["<id1>", "<id2>"]` |

Notes:
- The **provider id is the sender** (`from`) — agents register *themselves*.
- **search** replies to the requester; the provider list is a **JSON array** in
  `body`. Empty result ⇒ `[]` (not an error — the requester decides: retry, ask
  a parent DF, give up).
- DF reads the action from the UNL relation's source UW (`offer`/`seek`) and the
  service from its target UW.

## 4. Data model

```
registry: Map<service: String, providers: Set<String>>   // v1
```
- **Seeding:** DF may start empty, or be seeded from its `DATA` block (an initial
  registry) at spawn. v1: seed a couple of entries for the demo, plus dynamic
  register.
- **Later (semantic):** each entry also carries a *description/keywords*, and DF
  holds a **vector index** of those descriptions; search embeds the query and
  returns providers ranked by cosine similarity. This is why DF is its own crate
  — it will depend on an embedding/vector crate (reuse `unl-llm`'s `VectorIndex`)
  that AMS/PA never pull in.

## 5. Matching

- **v1:** exact service-name match.
- **v2 (hook):** semantic match over service descriptions (vector index). Search
  returns a *ranked list* — the table's `providers=` already supports multiple,
  ordered, results.

## 6. Hierarchy / federation (hook, not built v1)

DF may hold `parent: Option<AgentId>`. On a search **miss**:
- **forward** — query the parent DF and relay its answer, or
- **redirect** — reply `providers=` plus `redirect=<parentDF>` so the requester
  asks upward itself.

v1 is local-only; the `parent` field + the documented behavior are the seam.

## 7. Edge cases

| Case | Behavior |
|---|---|
| unknown service | reply with empty `providers=` |
| multiple providers | return all (ranked once semantic) |
| duplicate register | idempotent — provider already in the set |
| deregister absent | no-op, still ack |
| malformed `body` | ignore the bad field; reply with empty/derived result |

## 8. Crate layout

```
agents/df/
  Cargo.toml         # df-agent, rlib; deps: unl-agent (+ later: embedding/vector)
  src/lib.rs         # struct Df { registry, parent }  impl unl_agent::Agent
```
Registered in the node via `process::native_agent("df")`.

## 9. Test plan

- register then search returns the provider.
- search for an unknown service returns empty.
- duplicate register is idempotent; deregister removes.
- (v2) semantic search ranks the right provider first.

## 10. Decisions (resolved)

- **Payload** — UNL content (action + service) + optional JSON body (structured
  results). ✓
- **Verb form** — real UNL. ✓
- **Seeding** — seed initial entries from the `DATA` block + allow runtime
  register. ✓
- **v1 scope** — register + search only (deregister / federation / vector
  deferred, hooks kept). ✓

Remaining confirm before build: the UNL verb convention
(`offer` / `seek` → reply `registered` / `provide`) and service-as-single-UW for
v1.
