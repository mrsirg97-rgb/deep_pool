# DeepPool: Immutable Constant-Product Pool Protocol

## Overview

DeepPool is a permissionless, constant-product AMM for Solana. Each pool pairs a Token-2022 token with native SOL (no WSOL). A 0.25% swap fee compounds 100% back into the pool — no protocol fees, no admin, no extraction. Once created, a pool cannot be modified or shut down.

20% of all LP tokens are permanently locked on every deposit (create + add liquidity). The locked LP is minted to the pool PDA — an address that can never call `remove_liquidity`. This means pools can only get deeper over time: swap fees compound, locked LP accumulates, and depth is a ratchet that never goes backwards.

Designed as the liquidity layer for Torch Market, replacing the Raydium dependency. Compatible with the depth-anchored risk model — pool SOL reserve is readable on-chain for margin risk gating.

## Constraint

Everything on-chain. No governance. No protocol fee. Pools are immutable on creation — no admin can modify, pause, or close an existing pool. The program itself is upgradeable (bug fixes, new features), but upgrades cannot affect existing pool state or assets.

## Design Principles

1. **Immutable pools** — no owner, no authority, no modification after creation
2. **Permissionless** — anyone can create a pool, add liquidity, or swap
3. **Self-deepening** — 0.25% fee stays in the pool, K grows with every trade
4. **20% LP lock** — every deposit permanently locks 20% of liquidity in the pool
5. **Token-2022 native** — no WSOL, no token wrapping, handles transfer fees natively
6. **Minimal** — four instructions, three account types, two invariants

## Instructions

### 1. `create_pool`

Initialize a new pool for a Token-2022 mint paired with native SOL.

**Inputs:**
- `initial_token_amount` — tokens to deposit
- `initial_sol_amount` — SOL lamports to deposit

**Flow:**
1. Derive pool PDA from token mint
2. Create pool state, token vault, LP mint
3. Transfer tokens from creator to vault (measure net received)
4. Transfer SOL from creator to pool PDA
5. Compute initial LP supply: `sqrt(sol_amount * net_tokens) - MIN_LIQUIDITY`
6. Mint 80% LP to creator, 20% LP to pool PDA (permanently locked)
7. Record initial reserves, pool is live

**Constraints:**
- One pool per token mint (PDA enforced)
- `initial_sol_amount >= MIN_INITIAL_SOL` (0.1 SOL)
- `initial_token_amount >= MIN_INITIAL_TOKENS` (1 token)
- Token must be Token-2022

### 2. `add_liquidity`

Deposit SOL + tokens proportionally, receive LP tokens.

**Inputs:**
- `token_amount` — tokens to deposit
- `max_sol_amount` — maximum SOL willing to deposit (slippage protection)

**Flow:**
1. Compute required SOL: `sol_required = token_amount * sol_reserve / token_reserve`
2. Slippage check: `sol_required <= max_sol_amount`
3. Transfer tokens to vault, SOL to pool PDA
4. Compute LP: `lp_amount = lp_supply * net_tokens / token_reserve`
5. Mint 80% LP to provider, 20% LP to pool PDA (permanently locked)

**Constraints:**
- Pool must exist with non-zero reserves
- Proportional deposit enforced — no single-sided adds
- Provider receives 80% of LP minted; 20% locked

### 3. `remove_liquidity`

Burn LP tokens, receive proportional share of SOL + tokens.

**Inputs:**
- `lp_amount` — LP tokens to burn
- `min_sol_out` — minimum SOL to receive
- `min_tokens_out` — minimum tokens to receive

**Flow:**
1. Compute proportional share: `sol_out = lp_amount * sol_reserve / lp_supply`
2. Slippage checks
3. Ensure pool retains `sol_remaining > 0 && tokens_remaining > 0`
4. Burn LP, transfer SOL + tokens to provider

**Key behavior:** Because the pool PDA holds 20% of all ever-minted LP, `lp_supply` is larger than the sum of all user-held LP. This means no set of users can collectively redeem 100% of reserves.

### 4. `swap`

Exchange SOL for tokens or tokens for SOL.

**Inputs:**
- `amount_in` — amount of input asset
- `minimum_out` — slippage protection
- `buy` — true = SOL→Token, false = Token→SOL

**Flow (buy):**
1. Fee: `fee = amount_in * 25 / 10000` (0.25%)
2. Output: `tokens_out = (effective_in * token_reserve) / (sol_reserve + effective_in)`
3. Fee SOL stays in pool — K increases

**Invariant:** `K_new >= K_old` after every swap. K never decreases.

## Constants

```
POOL_SEED         = "deep_pool"
VAULT_SEED        = "pool_vault"
LP_MINT_SEED      = "pool_lp_mint"
SWAP_FEE_BPS      = 25          // 0.25%
FEE_DENOMINATOR   = 10000
LP_BURN_BPS       = 2000        // 20% of LP locked per deposit
MIN_LIQUIDITY     = 1000        // locked on first deposit
MIN_INITIAL_SOL   = 100_000_000 // 0.1 SOL
MIN_INITIAL_TOKENS = 1_000_000  // 1 token (6 decimals)
```

## LP Lock Economics

The 20% LP lock creates aligned incentives:

- **LPs are committed** — losing 20% per deposit means you're in for the long term
- **Promote, don't exit** — the best way to increase your LP value is to drive swap volume
- **Bank run protection** — even if everyone exits, permanent reserves remain
- **Compounding lock** — repeated add/remove cycles increase locked depth: `0.8^n` retained after n cycles
- **Early LPs rewarded** — they get 80% of LP on a shallow pool; as the pool deepens their share appreciates from both fees and subsequent LPs locking 20%

## Token-2022 Transfer Fee Handling

Torch tokens have a 0.04% transfer fee. On sells, the vault receives less than the user sends. Swap math uses **net received**:

```
net_received = vault_balance_after - vault_balance_before
```

Same for `add_liquidity` — LP computed from net amount.

## Integration with Torch

- **Migration:** Torch CPIs into `create_pool`, then burns ALL LP (permanent liquidity)
- **Depth model:** Reads `pool_pda.lamports() - rent` for SOL reserve
- **Fee harvesting:** `swap_fees_to_sol` CPIs into `swap`
- **Community LP:** After migration, anyone can add liquidity (receives 80% LP, 20% locked)

## File Structure

```
programs/deep_pool/src/
  lib.rs              — entrypoint, 4 instructions
  state.rs            — Pool account struct
  constants.rs        — seeds, fee rate, LP burn rate, minimums
  error.rs            — error codes
  kani_proofs.rs      — 16 formal verification proofs
  instructions/
    create_pool.rs    — pool init + first LP mint (80/20 split)
    add_liquidity.rs  — proportional deposit + LP mint (80/20 split)
    remove_liquidity.rs — LP burn + proportional withdrawal
    swap.rs           — buy/sell with fee compounding
```
