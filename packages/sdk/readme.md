# DeepPool SDK

TypeScript SDK for [DeepPool](https://github.com/mrsirg97-rgb/deep_pool) — a constant-product pool protocol on Solana. 0.25% swap fee, 100% back to liquidity. Permanent LP locks on every deposit. Pools that only get deeper.

## Install

```bash
pnpm add deeppoolsdk
```

Peer dependency: `@solana/web3.js ^1.98.0`

## How It Works

```
1. Read pool state   →  getPool
2. Get a quote       →  getSwapQuote (client-side, no RPC)
3. Build a tx        →  buildSwapTransaction
4. Sign and send     →  your wallet / keypair
```

## Operations

### Read

| Function | Description |
|----------|-------------|
| `getPool(connection, tokenMint)` | Pool state: reserves, price, swap count |
| `getSwapQuote(solReserve, tokenReserve, amountIn, buy, transferFeeBps?)` | Expected output, fee, price impact. Client-side — no RPC call |

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
| `getPoolPda(tokenMint)` | Pool state PDA |
| `getVaultPda(pool)` | Token vault PDA |
| `getLpMintPda(pool)` | LP token mint PDA |

## Example

```typescript
import { Connection, LAMPORTS_PER_SOL } from '@solana/web3.js'
import { getPool, getSwapQuote, buildSwapTransaction } from 'deeppoolsdk'

const connection = new Connection('https://api.devnet.solana.com')
const mint = 'YOUR_TOKEN_MINT'

// Read pool
const pool = await getPool(connection, mint)
console.log(`Price: ${pool.price} SOL/token`)
console.log(`SOL reserve: ${pool.solReserve / LAMPORTS_PER_SOL}`)

// Quote a buy
const quote = getSwapQuote(
  pool.solReserve,
  pool.tokenReserve,
  1 * LAMPORTS_PER_SOL, // 1 SOL
  true, // buy
)
console.log(`Expected tokens: ${quote.amountOut}`)
console.log(`Price impact: ${quote.priceImpactPercent}%`)

// Build transaction
const { transaction } = await buildSwapTransaction(connection, {
  user: walletAddress,
  tokenMint: mint,
  amountIn: 1 * LAMPORTS_PER_SOL,
  minimumOut: Math.floor(quote.amountOut * 0.99), // 1% slippage
  buy: true,
})
// sign and send
```

## Key Properties

- **LP locks** — creators lock 20%, community LPs lock 7.5%. Permanently held by pool PDA. Pools only get deeper.
- **Self-deepening** — 0.25% swap fee compounds into reserves. K only grows.
- **No freeze authority** — LP tokens can never be frozen by anyone.
- **Token-2022 native** — no WSOL wrapping.
- **Native SOL** — SOL reserve is pool PDA lamports, not a token account.
- **Immutable pools** — no admin, no fee switch, no close.
- **No protocol fee** — 0% extraction. All fees stay in the pool.
- **Formally verified** — 16 Kani proofs cover swap math, LP math, K invariant, LP locks.

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

## Testing

```bash
# Mainnet fork (Surfpool)
surfpool start --network mainnet --no-tui
npx tsx tests/test_e2e.ts
```

## Links

- [Design](../../docs/design.md)
- [Verification](../../docs/verification.md)
- [Audit](../../docs/audit.md)
- Built for [torch.market](https://torch.market)
- Program ID: `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`

## License

MIT
