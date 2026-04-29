# DeepPool SDK

TypeScript SDK for [DeepPool](https://github.com/mrsirg97-rgb/deep_pool) — a constant-product pool protocol on Solana. 0.25% swap fee, 100% back to liquidity. Permanent LP locks on every deposit. Pools that only get deeper.

## Install

```bash
pnpm add deeppoolsdk
```

Peer dependency: `@solana/web3.js ^1.98.0`

## How It Works

```
1. Read pool state   →  getPool (RPC, or indexer-first with { indexer })
2. Get a quote       →  getSwapQuote (client-side, no RPC)
3. Build a tx        →  buildSwapTransaction
4. Sign and send     →  your wallet / keypair
5. Subscribe live    →  createIndexerBus (optional, indexer-backed)
```

## Operations

### Read

| Function | Description |
|----------|-------------|
| `getPool(connection, tokenMint, config, options?)` | Pool state by `(config, mint)`. Indexer-first with RPC fallback when `options.indexer` is set. |
| `getPoolByAddress(connection, poolAddress, options?)` | Pool state by PDA. Same indexer-first behavior. |
| `getPoolsForMint(connection, tokenMint)` | Every pool for a mint, across all configs. RPC-only. |
| `getSwapQuote(solReserve, tokenReserve, amountIn, buy, transferFeeBps?)` | Expected output, fee, price impact. Pure compute — no RPC. |

`ReadOptions = { indexer?: string }`. On any indexer failure (network, non-200, malformed) the call silently falls back to RPC.

### Indexer-only Reads

These have no RPC equivalent — chain RPCs can't answer "all swaps for pool X since time T". They throw if the indexer is unreachable.

| Function | Description |
|----------|-------------|
| `getSwapHistory({ indexer, poolId?, user?, tokenMint?, since?, before?, limit? })` | Filtered swap history. |
| `getLiquidityHistory({ indexer, poolId?, provider?, tokenMint?, isAdd?, since?, before?, limit? })` | Filtered add/remove history. |

### Live Events

`createIndexerBus(indexerUrl)` opens a single WebSocket per process and dispatches typed frames to subscribers. Auto-reconnects on disconnect and gap-fills via REST so consumers see every event that landed during the disconnect window.

| Method | Description |
|--------|-------------|
| `bus.on(kind, handler)` | Subscribe to `'pool' \| 'swap' \| 'liquidity' \| 'reserves'`. Returns unsubscribe. |
| `bus.onConnectionChange(handler)` | `'connected' \| 'disconnected'` transitions. |
| `bus.close()` | Close socket and clear all handlers. |

Frames are a discriminated union — `BroadcastFrame = { kind: 'pool', ...IndexerPoolRow } | { kind: 'swap', ...SwapRow } | ...`.

### Pool Management

| Function | Description |
|----------|-------------|
| `buildCreatePoolTransaction(connection, params)` | Create pool with initial SOL + tokens. 80% LP to creator, 20% locked permanently. |
| `buildAddLiquidityTransaction(connection, params)` | Proportional deposit. 92.5% LP to provider, 7.5% locked permanently. |
| `buildRemoveLiquidityTransaction(connection, params)` | Burn LP tokens. Receive proportional SOL + tokens. Pool retains locked reserves. |

### Trading

| Function | Description |
|----------|-------------|
| `buildSwapTransaction(connection, params)` | Buy (SOL→Token) or sell (Token→SOL). 0.25% fee compounds into pool. |

### PDA Derivation

| Function | Description |
|----------|-------------|
| `getPoolPda(config, tokenMint)` | Pool state PDA (namespaced by config). |
| `getVaultPda(pool)` | Token vault PDA. |
| `getLpMintPda(pool)` | LP token mint PDA. |
| `getEventAuthorityPda()` | Anchor `emit_cpi!` authority PDA. |

### Events

| Function | Description |
|----------|-------------|
| `parseEvents(connection, signature)` | Decode `PoolCreated`, `LiquidityAdded`, `LiquidityRemoved`, `SwapExecuted` from a confirmed tx's inner instructions. |

## Example — RPC Read

```typescript
import { Connection } from '@solana/web3.js'
import { getPool, getSwapQuote, buildSwapTransaction, LAMPORTS_PER_SOL } from 'deeppoolsdk'

const connection = new Connection('https://api.devnet.solana.com')
const mint = 'YOUR_TOKEN_MINT'
const config = 'YOUR_CONFIG_PUBKEY'

const pool = await getPool(connection, mint, config)
if (!pool) throw new Error('pool not found')

const quote = getSwapQuote(pool.solReserve, pool.tokenReserve, 1 * LAMPORTS_PER_SOL, true)

const { transaction } = await buildSwapTransaction(connection, {
  user: walletAddress,
  tokenMint: mint,
  amountIn: 1 * LAMPORTS_PER_SOL,
  minimumOut: Math.floor(quote.amountOut * 0.99),
  buy: true,
})
```

## Example — Indexer-First Read

```typescript
import { getPool, getSwapHistory } from 'deeppoolsdk'

const indexer = process.env.NEXT_PUBLIC_INDEXER_URL // e.g. http://localhost:8080

// Pool state straight from the indexer; falls back to RPC on failure.
const pool = await getPool(connection, mint, config, { indexer })

// History is indexer-only — no RPC equivalent.
const recentSwaps = await getSwapHistory({
  indexer,
  poolId: 1,
  since: new Date(Date.now() - 24 * 60 * 60 * 1000),
  limit: 100,
})
```

## Example — Live Event Bus

```typescript
import { createIndexerBus } from 'deeppoolsdk'

const bus = createIndexerBus('http://localhost:8080')

const offPool = bus.on('pool', (frame) => {
  console.log('new pool', frame.pubkey, 'token', frame.token_mint)
})

const offSwap = bus.on('swap', (frame) => {
  console.log('swap', frame.is_buy ? 'BUY' : 'SELL', frame.amount_in_net, '→', frame.amount_out_net)
})

bus.onConnectionChange((s) => console.log('bus:', s))

// later
offPool(); offSwap(); bus.close()
```

Outside a browser (SSR, Node tests) `createIndexerBus` returns a no-op bus.

## Key Properties

- **LP locks** — creators lock 20%, community LPs lock 7.5%. Permanently held by pool PDA. Pools only get deeper.
- **Self-deepening** — 0.25% swap fee compounds into reserves. K only grows.
- **No freeze authority** — LP tokens can never be frozen by anyone.
- **Token-2022 native** — no WSOL wrapping.
- **Native SOL** — SOL reserve is pool PDA lamports, not a token account.
- **Immutable pools** — no admin, no fee switch, no close.
- **No protocol fee** — 0% extraction. All fees stay in the pool.
- **Formally verified** — Kani proofs cover swap math, LP math, K invariant, LP locks.

## Constants

| Parameter | Value |
|-----------|-------|
| Swap fee | 0.25% (25 bps) |
| Creator LP lock | 20% (2000 bps) |
| Provider LP lock | 7.5% (750 bps) |
| Protocol fee | 0% |
| Min initial SOL | 0.1 SOL |
| Min initial tokens | 1 token (6 decimals) |
| MIN_LIQUIDITY | 1000 (locked on first deposit) |
| LP decimals | 6 |
| `POOL_ACCOUNT_SIZE` | 153 bytes |

Re-exports: `LAMPORTS_PER_SOL`, `TOKEN_2022_PROGRAM_ID`, `PROGRAM_ID`, `SWAP_FEE_BPS`, `FEE_DENOMINATOR`, `POOL_ACCOUNT_SIZE`.

## Testing

```bash
# Mainnet fork (Surfpool) — pure SDK + program path
pnpm test

# Devnet end-to-end against deployed program
pnpm test:devnet

# Local docker stack (program + indexer + db)
docker compose --profile full up -d
pnpm test:local

# WebSocket bus against the local indexer
pnpm test:bus
```

The local and bus suites need `INDEXER_URL` (default `http://localhost:8080`) and a funded keypair at `KEYPAIR_PATH`.

## Links

- [Design](../../docs/design.md)
- [Verification](../../docs/verification.md)
- [Audit](../../docs/audit.md)
- [Events](../../docs/events.md)
- [Indexer](../../docs/indexer.md)
- Built for [torch.market](https://torch.market)
- Program ID: `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`

## License

MIT
