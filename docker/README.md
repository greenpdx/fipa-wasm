# FIPA nodes in Docker — cross-node book-buy over IP

Each agent runs in its **own container** (its own IP) as a `mesh-node`. The node
is the engine; the **agent is added** via `FIPA_AGENT`. Nodes talk over TCP/IP,
reaching each other by Docker service name (→ container IP) — the same address
AMS hands out.

## Run

```bash
# 1. Build the base node image (once)
docker build -t fipa-node:base -f docker/Dockerfile .

# 2. Bring up the five-node platform
docker compose -f docker/docker-compose.yml up
```

Watch the `ba` container — it prints:

```
[ba] RESULT → 'result' : obj(bought, LtG) {}
```

## What happens

```
ams  (white pages)   df (yellow pages)   pa (escrow)   bs (seller)   ba (buyer)
```

1. `bs` **registers** at startup: `offer bookselling` → DF, `bind <uuid>→bs:9000` → AMS.
2. `ba` kicks off: `seek bookselling` → DF → the seller's **UUID**.
3. `ba`: `locate <uuid>` → AMS → `bs:9000` (the address); the node caches it.
4. `ba` → `bs` catalog → picks *Limits to Growth* → `reserve` → `pa`.
5. `pa` holds, notifies `bs` (resolving `bs`'s address via AMS); `bs` `accept`s;
   `pa` releases; `bs` ships `deliver` to `ba`. ✓ bought.

## Identity

- **Infra** (`ams`/`df`/`pa`/`bs`): UUID is **minted at first spawn and persisted**
  to the `/data` volume — stable across `docker restart`.
- **Buyer** (`ba`): UUID **pinned** via `FIPA_UUID` so `pa` can be pre-funded in
  the compose file. Drop `FIPA_UUID` to mint+persist like the others.

## Addressing (no static UUID map)

- **bootstrap**: well-known `ams`/`df`/`pa` addresses come from env.
- **return address**: every message carries the sender's address; replies route back.
- **AMS resolution**: an unknown UUID's address is fetched from the AMS node.

## Local equivalent (no Docker)

`cargo run --bin book-cluster` runs the same five nodes over TCP on `127.0.0.1`
(different ports) — identical protocol, handy for development.
