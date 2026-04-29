# DeepPool Indexer

## Goal

Capture every `PoolCreated` / `SwapExecuted` / `LiquidityAdded` / `LiquidityRemoved` event from on-chain via Helius Laserstream, persist to Postgres, expose via HTTP/WS. Pool state, price history, TVL, and LP-share-value reconstruction are all derivable from the persisted event stream — no on-chain reads on the read path.

## Architecture

Two halves, sharing the database:

```
 ┌──────────────┐  block stream   ┌──────────────┐  per-block tx  ┌──────────────┐
 │ Laserstream  │ ───────────────▶│  Decoder     │ ──────────────▶│  Postgres    │
 │  (gRPC)      │  filtered to    │  + Writer    │  events +      │              │
 └──────────────┘  deep_pool      │  task        │  reserves +    │              │
                                  └──────┬───────┘  checkpoint    └──────┬───────┘
                                         │ post-COMMIT                    │
                                         ▼                                ▼
                                  ┌──────────────┐  REPEATABLE READ  ┌──────────────┐
                                  │  WS /events  │  txn per request  │  axum API    │
                                  │  broadcast   │ ◀─────────────────│  (services)  │
                                  └──────────────┘                   └──────────────┘
```

**Stream side** (`stream/`): Laserstream → decoder (4 event types) → writer (one txn per block, post-COMMIT broadcast). Modeled on `~/Projects/metadao-challenge/indexer`.

**API side** (`api.rs` + `services/` + `domain/`): axum HTTP/WS, per-request transaction at REPEATABLE READ isolation, service DAG composed lazily from a request context.

## Request lifecycle (API side)

1. Middleware opens `BEGIN ISOLATION LEVEL REPEATABLE READ` per incoming request, attaches `RequestCtx<'tx>` to the request extensions.
2. `RequestCtx` owns the `sqlx::Transaction<'_, Postgres>` plus a per-request `Cache` (HashMaps keyed by id/pubkey).
3. Handler accesses services via `ctx.pools()`, `ctx.swaps()`, etc. Each call returns a tiny wrapper holding `&mut RequestCtx` — construction is free, no allocation, no heap. Work happens only when service methods are invoked. **This is the lazy-loading pattern.**
4. Service methods check the per-request cache before issuing SQL. Same pool looked up twice in a request = one query.
5. On handler success, middleware commits (read-only commit is cheap); on error, rollback.

**Why REPEATABLE READ:** snapshot consistency across all queries in a request without taking SHARED row/table locks. Writes (the indexer task) and reads (API requests) don't block each other; MVCC handles the consistency.

## Service DAG

Domain layer = thin CRUD per table. Services compose domain calls + use the per-request cache.

```
┌──────────────┐  composes   ┌──────────────────┐  composes   ┌──────────────────┐
│ SwapService  │ ───────────▶│   PoolService    │◀─────────── │ LiquidityService │
└──────────────┘             │ cache: pool_id   │             └──────────────────┘
                             │      + pubkey    │
                             └──────────────────┘

┌──────────────────┐  standalone — called directly by handlers
│ ReservesService  │  (the pool-detail endpoint composes pool + reserves
└──────────────────┘   at the handler layer, not service-to-service)
```

`PoolService` is the only composition target — both `SwapService` and `LiquidityService` route through it for token-mint filters. `ReservesService` and `PoolService` are leaves (no inter-service deps); the cache lives on `PoolService` because pubkey-and-id lookups dominate, and on `ReservesService` for "current state per pool."

**Domain methods (per table)**: `set(rows)`, `get(id)`, `list(filter)`, `del(ids)`. Same shape across pool, swap, liquidity, reserves.

**Service methods** build filters and delegate. Example — `SwapService::for_token_mint(mint)`:
1. `ctx.pools().for_token_mints([mint])` → `Vec<Arc<PoolRow>>`, cached by pool_id and pubkey.
2. Map to `pool_ids: Vec<i32>`.
3. `swap::list(tx, SwapFilter { pool_ids: Some(pool_ids), ..Default::default() })`.
4. Return as `Vec<Arc<SwapRow>>`.

No JOIN, no view, no DB-side aggregation. The "join" is two domain queries with a `HashMap<i32, Arc<PoolRow>>` in between.

## Schema

Replaces the current `db/01-schema.sql`. Fixes: BIGINT for u64-range values, correct Postgres FK syntax, missing comma, idempotency keys, checkpoint table, two-direction liquidity table.

```sql
CREATE TABLE pools (
    pool_id            SERIAL PRIMARY KEY,
    pubkey             TEXT NOT NULL UNIQUE,
    config             TEXT NOT NULL,
    token_mint         TEXT NOT NULL,
    lp_mint            TEXT NOT NULL,
    creator            TEXT NOT NULL,
    sol_initial        BIGINT NOT NULL,
    tokens_initial     BIGINT NOT NULL,
    lp_supply_initial  BIGINT NOT NULL,
    slot               BIGINT NOT NULL,
    signature          TEXT NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL
);
CREATE INDEX pools_token_mint_idx ON pools(token_mint);
CREATE INDEX pools_creator_idx    ON pools(creator);

CREATE TABLE reserves (
    pool_id        INT PRIMARY KEY REFERENCES pools(pool_id),
    sol_reserve    BIGINT NOT NULL,
    token_reserve  BIGINT NOT NULL,
    lp_supply      BIGINT NOT NULL,
    last_slot      BIGINT NOT NULL,
    last_updated   TIMESTAMPTZ NOT NULL
);

CREATE TABLE swaps (
    swap_id              SERIAL PRIMARY KEY,
    pool_id              INT NOT NULL REFERENCES pools(pool_id),
    user_pk              TEXT NOT NULL,
    sol_source           TEXT NOT NULL,
    is_buy               BOOLEAN NOT NULL,
    amount_in_gross      BIGINT NOT NULL,
    amount_in_net        BIGINT NOT NULL,
    amount_out_gross     BIGINT NOT NULL,
    amount_out_net       BIGINT NOT NULL,
    fee                  BIGINT NOT NULL,
    sol_reserve_after    BIGINT NOT NULL,
    token_reserve_after  BIGINT NOT NULL,
    slot                 BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    inner_ix_idx         INT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL,
    UNIQUE (signature, inner_ix_idx)
);
CREATE INDEX swaps_pool_created_idx ON swaps(pool_id, created_at DESC);
CREATE INDEX swaps_user_created_idx ON swaps(user_pk, created_at DESC);

CREATE TABLE liquidity_events (
    liquidity_id         SERIAL PRIMARY KEY,
    pool_id              INT NOT NULL REFERENCES pools(pool_id),
    provider             TEXT NOT NULL,
    is_add               BOOLEAN NOT NULL,
    sol_amount_gross     BIGINT NOT NULL,
    sol_amount_net       BIGINT NOT NULL,
    tokens_amount_gross  BIGINT NOT NULL,
    tokens_amount_net    BIGINT NOT NULL,
    lp_user_amount       BIGINT NOT NULL,  -- lp_to_provider on add, lp_burned on remove
    lp_locked            BIGINT NOT NULL,  -- 0 on remove
    lp_supply_after      BIGINT NOT NULL,
    slot                 BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    inner_ix_idx         INT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL,
    UNIQUE (signature, inner_ix_idx)
);
CREATE INDEX liquidity_pool_created_idx     ON liquidity_events(pool_id, created_at DESC);
CREATE INDEX liquidity_provider_created_idx ON liquidity_events(provider, created_at DESC);

CREATE TABLE indexer_state (
    id INT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    last_processed_slot BIGINT NOT NULL
);
```

**Why BIGINT (i64), not NUMERIC:** signed i64 holds up to 9.2e18; max possible Solana lamports are ~4.3e15 (4.3M SOL total supply); typical token supplies (1e15) fit. NUMERIC would be needed only for adversarial mints with supply near u64::MAX, which isn't a real case. Decoders cast u64 → i64 with debug-asserted bounds.

**Why one `liquidity_events` table:** unifies adds + removes for chronological per-pool/per-provider listings (single ORDER BY). Direction inferred from `is_add`. The "unused" outbound fields on adds (`sol_amount_net` is the inbound) reuse columns by changing semantics, with `is_add` driving interpretation.

## Stream pipeline

Per block, **one Postgres transaction**:

```
BEGIN
  for each decoded event in (tx_index, inner_ix_idx) order:
    PoolCreated     → INSERT pools (ON CONFLICT (pubkey) DO NOTHING)
                      INSERT reserves (ON CONFLICT (pool_id) DO UPDATE)  -- bootstraps
    SwapExecuted    → INSERT swaps (ON CONFLICT (signature, inner_ix_idx) DO NOTHING)
                      UPDATE reserves SET sol_reserve, token_reserve, last_slot, last_updated
    Liquidity*      → INSERT liquidity_events (ON CONFLICT (...) DO NOTHING)
                      UPDATE reserves (sol, token, lp_supply, ...)
  UPDATE indexer_state SET last_processed_slot = <block.slot>
COMMIT
→ post-COMMIT WS broadcast for the rows actually inserted
```

**Atomicity:** crash mid-block = next start replays from `last_processed_slot - K` (K = 32 slots), idempotent UPSERTs make it a no-op.

**Order within a block matters** because `PoolCreated` must precede any swap/liquidity row referencing the same `pool_id`. Sort by `(tx_index, inner_ix_idx)` before processing.

**Reserves update is in-tx with the event**, no triggers. Triggers were considered and rejected: invisible to readers of the indexer code, harder to test, and the same atomic-write pattern handles it explicitly.

## Decoder

Mirror `~/Projects/metadao-challenge/indexer/src/stream/decoder.rs`. Four event types instead of one:

```rust
pub fn discriminators() -> EventDiscriminators {
    EventDiscriminators {
        pool_created:      event_discriminator("PoolCreated"),
        swap_executed:     event_discriminator("SwapExecuted"),
        liquidity_added:   event_discriminator("LiquidityAdded"),
        liquidity_removed: event_discriminator("LiquidityRemoved"),
    }
}

pub fn try_decode_event(data: &[u8], discs: &EventDiscriminators) -> Result<DeepPoolEvent, DecodeError>
```

Returns a `DeepPoolEvent` enum. Each variant maps to a Borsh struct in `contracts.rs` matching the IDL field order.

**No V1 fallback** initially — the deep_pool program ships with these events from the start, no historical re-shape. Add fallback logic later if event layouts ever change.

## Backfill

Same shape as metadao-challenge backfill: paginate `getSignaturesForAddress(programId)`, fetch each tx via `getTransaction`, decode through the same `try_decode_event` path, UPSERT through the same conflict keys. Never touches `last_processed_slot` (that's the live indexer's checkpoint).

## API surface (v1)

- `GET /api/pools?token_mint=&creator=` — list pools, optional filters
- `GET /api/pools/:pubkey` — pool detail (joins reserves in service layer)
- `GET /api/swaps?pool_id=&user=&since=&limit=` — swap history
- `GET /api/liquidity?pool_id=&provider=&since=&limit=` — liquidity history
- `WS /events` — live stream of inserted rows post-COMMIT

All HTTP routes wrapped by the REPEATABLE-READ middleware.

## Out of scope (v1)

- Read replicas / multi-region.
- Aggregated analytics (TVL/volume/OHLCV endpoints) — derivable from the event stream, defer until a consumer asks.
- Auth on the API.
- Multi-program indexing.
- Cross-reorg reconciliation — devnet `confirmed` is effectively final; the 32-slot replay margin covers drift.
