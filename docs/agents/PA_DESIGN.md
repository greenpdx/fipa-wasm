# Payment Agent (PA) — design

PA settles payment between a buyer and a seller using an **escrow / hold**: it
**reserves** (holds) the buyer's funds, then **releases** them to the seller on
**accept** or back to the buyer on **deny**, issuing **receipts** throughout. It
is the money side of the book purchase; the seller (BS) runs the mirror-image
*book* reservation on its own side.

## 1. The six verbs as a state machine

```
                 reserve (buyer)
                      │
        insufficient ╱ ╲ ok
              deny ◀╱   ╲▶  HOLD ──── receipt: "held"
                          │
            ┌─────────────┼──────────────┐
       accept (seller)               deny (cancel)
            │                             │
     release → seller             release → buyer
       state: PAID                 state: CANCELLED
       receipt: "paid"  ×2         receipt: "cancelled" ×2
```

| Verb | direction | meaning |
|---|---|---|
| **reserve** | in (buyer) | escrow the funds for an order |
| **hold** | *state* | funds escrowed, awaiting accept/deny (shown in the receipt) |
| **accept** | in (seller) | complete: **release** the hold to the seller |
| **deny** | in (buyer/seller); also *out* | cancel: **release** the hold back to the buyer; *or* PA's rejection of a `reserve` (insufficient funds) |
| **release** | *PA action* | the fund transfer — to seller on accept, to buyer on deny |
| **receipt** | out | proof of outcome (`held` / `paid` / `cancelled`), to buyer **and** seller |

## 2. Where PA sits in the book flow

```
BA → PA : reserve LtG {seller:bookSeller, amount:999}   PA → BA : receipt {held}
                                                        PA → BS : receipt {held, buyer:BA}
(BS reserves the book; BS ships)
BS → PA : accept LtG        PA → BA,BS : receipt {paid}    (funds released to BS)
   …or on cancel/timeout…
   → PA : deny LtG          PA → BA,BS : receipt {cancelled} (funds back to BA)
```

## 3. Message model

`(from, unl, body)` — UNL carries the verb + the order id; JSON body carries the
structured terms (seller, amount).

| Action | in: `unl` / `body` | PA does |
|---|---|---|
| **reserve** | `obj(reserve, <order>)` / `{"seller":"<id>","amount":<n>}` | buyer = sender; if `balance[buyer] ≥ amount`: hold it; else deny |
| **accept** | `obj(accept, <order>)` / — | release the hold to the seller (paid) |
| **deny** | `obj(deny, <order>)` / — | release the hold back to the buyer (cancelled) |

PA looks up `buyer`/`seller`/`amount` from the stored hold on accept/deny, so
those messages only name the `<order>`.

Replies (UNL / JSON):
- **receipt** — `obj(receipt, <order>)` / `{"status":"held"|"paid"|"cancelled","amount":<n>,…}`.
- **deny** (rejection) — `obj(deny, <order>)` / `{"reason":"insufficient"}`.

## 4. Data model

```
ledger : Map<account, u64>     # available balances (integer minor units, e.g. cents)
holds  : Map<order, Hold>      # Hold { buyer, seller, amount, state }
enum HoldState { Held, Paid, Cancelled }
```
- **reserve**: `ledger[buyer] -= amount`; `holds[order] = Hold{…, Held}`.
- **accept**: `ledger[seller] += amount`; `state = Paid`.
- **deny**:   `ledger[buyer] += amount`; `state = Cancelled` (refund).

Seeded from the `DATA` block (`on_seed`):
```json
{ "ledger": { "BA": 10000, "bookSeller": 0 } }
```

## 5. Receipts / notifications

- **reserve** → `receipt{held}` to the buyer **and** to the seller (so BS knows
  payment is secured and can reserve/ship the book — "PA confirms payment to BS").
- **accept** → `receipt{paid}` to buyer and seller.
- **deny**   → `receipt{cancelled}` to buyer and seller.

## 6. Edge cases

| Case | Behavior |
|---|---|
| insufficient funds on reserve | `deny{insufficient}` to buyer; no hold created |
| duplicate reserve for an order | reject (order already held) |
| accept/deny on unknown order | ignore (or a `receipt{unknown}`) |
| accept/deny on a finalized hold | no-op; receipt echoes the current state |
| bad amount / fields | ignore the message |

## 7. Security

PA handles money, so identity and durability are first-class.

**Baked into v1 (cheap, correct now — enforceable once `from` is authenticated):**
- **Authorization** — `accept` only from `hold.seller`; `deny` only from the
  hold's buyer or seller. Checked against `ctx.from()`.
- **Idempotency** — order ids are unique; a duplicate `reserve` is rejected; the
  one-way state machine (`Held → Paid|Cancelled`) blunts `accept`/`deny` replay.
- **Input validation** — amounts are bounded non-negative integers; no overflow;
  malformed messages ignored.
- **Scoped receipts** — only to the hold's actual buyer/seller.

**Deferred (with hooks), by layer:**
- *FIPA/node:* **authenticate `from`** (signed messages / authenticated channels)
  — authorization is only meaningful once this exists; transport integrity (TLS)
  for cross-node; rate-limiting. *(Process isolation + `rlimit` already done.)*
- *PA agent:* **durable ledger + holds** (a DB) — **without it a restart loses
  escrow**, since `ManagedAgent` rebuilds from the seed; audit log; timeout
  auto-`deny` of stale holds; `status` query.
- *end-to-end:* **sign** `(order, action, terms)` and PA's **receipts** for
  integrity + non-repudiation, independent of router trust.

**Sharpest risks today:** (1) `from` is unauthenticated → spoofed `accept`/`deny`;
(2) in-memory state → a restart loses held funds. Both are noted as the security
roadmap; v1 is a trusted-`from`, in-memory build.

## 7b. Other deferred

- Timeout release; `status` query; real money / multi-currency / fractional
  amounts.

## 8. Crate layout

```
agents/pa/
  Cargo.toml   # pa-agent, rlib; deps: unl-agent, unl-core, unl-parser, serde_json
  src/lib.rs   # struct Pa { ledger, holds }  impl unl_agent::Agent
```
Registered as `native_agent("pa")`.

## 9. Test plan

- reserve (sufficient) → `receipt{held}` to buyer + seller; buyer balance debited.
- reserve (insufficient) → `deny{insufficient}`; no balance change.
- reserve → accept → `receipt{paid}` ×2; seller credited, hold = Paid.
- reserve → deny → `receipt{cancelled}` ×2; buyer refunded, hold = Cancelled.
- accept/deny on unknown / already-finalized order → no-op.

## 10. Decisions to confirm before building

1. **Amount** — integer minor units (`amount: 999` = $9.99)? *(lean: yes.)*
2. **reserve notifies the seller** with `receipt{held}` (PA → BS), per "PA
   confirms payment to BS"? *(lean: yes.)*
3. **accept/deny name only the order** (PA has buyer/seller/amount from the
   hold)? *(lean: yes.)*
4. **v1 scope** — full escrow: **reserve + accept + deny** (with `hold`/`release`
   as the internal state/action, `receipt` as the reply)? Authorization, timeout
   release, and status query deferred?
5. **Verbs** — `reserve` / `accept` / `deny` in; `receipt` / `deny` out;
   statuses `held` / `paid` / `cancelled`. OK?
