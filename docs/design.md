# DeepPool: Immutable Constant-Product Pool Protocol

## Overview

DeepPool is a permissionless, constant-product AMM for Solana. Each pool pairs a Token-2022 token with native SOL (no WSOL). A 0.25% swap fee compounds 100% back into the pool — no protocol fees, no admin, no extraction. Once created, a pool cannot be modified or shut down.

LP tokens track proportional ownership. Torch burns its LP at migration (permanent base liquidity). Community members can add liquidity on top and remove it later. LP holders earn passively as fees deepen the pool — no staking, no claiming, no farming.

Designed as the liquidity layer for Torch Market, replacing the Raydium dependency. Compatible with the depth-anchored risk model (`get_depth_max_ltv_bps(pool_sol)`) — pool SOL reserve is readable on-chain for margin risk gating.

## Constraint

Everything on-chain. No governance. No protocol fee. Pools are immutable on creation — no admin can modify, pause, or close an existing pool. The program itself is upgradeable (bug fixes, new features), but upgrades cannot affect existing pool state or assets. Same pattern as Torch: program evolves, deployed state is permanent.

## Design Principles

1. **Immutable pools** — no owner, no authority, no modification after creation. Program is upgradeable; pools are not.
2. **Permissionless** — anyone can create a pool, add liquidity, or swap
3. **Self-deepening** — 0.25% fee stays in the pool, K grows with every trade
4. **Token-2022 native** — no WSOL, no token wrapping, handles transfer fees natively
5. **Minimal** — four instructions, three account types, one invariant

## Instructions

### 1. `create_pool`

Initialize a new pool for a Token-2022 mint paired with native SOL.

**Inputs:**
- `initial_token_amount` — tokens to deposit
- `initial_sol_amount` — SOL lamports to deposit

**Flow:**
1. Derive pool PDA from token mint
2. Create pool state account
3. Create token vault (Token-2022 ATA owned by pool PDA)
4. Create LP mint (Token-2022, authority = pool PDA)
5. Transfer tokens from creator to vault (measure net received after transfer fee)
6. Transfer SOL from creator to pool PDA
7. Compute initial LP supply: `sqrt(sol_amount * net_tokens_received)`
8. Mint LP tokens to creator
9. Record initial reserves, pool is live

**Constraints:**
- One pool per token mint (PDA enforced)
- Both amounts must be > 0
- Token must be Token-2022 (not legacy SPL)
- LP mint authority is the pool PDA — no one else can mint

### 2. `add_liquidity`

Deposit SOL + tokens proportionally, receive LP tokens.

**Inputs:**
- `token_amount` — tokens to deposit
- `max_sol_amount` — maximum SOL willing to deposit (slippage protection)

**Flow:**
1. Read current reserves (SOL = pool PDA lamports - rent, tokens = vault balance)
2. Compute required SOL for proportional deposit: `sol_required = token_amount * sol_reserve / token_reserve`
3. Slippage check: `sol_required <= max_sol_amount`
4. Transfer tokens from user to vault (measure net received)
5. Transfer proportional SOL from user to pool PDA
6. Compute LP tokens to mint: `lp_amount = lp_supply * net_tokens / token_reserve`
7. Mint LP tokens to user

**Constraints:**
- Pool must exist
- Both deposits must be > 0
- Proportional deposit enforced — no single-sided adds

### 3. `remove_liquidity`

Burn LP tokens, receive proportional share of SOL + tokens.

**Inputs:**
- `lp_amount` — LP tokens to burn
- `min_sol_out` — minimum SOL to receive (slippage protection)
- `min_tokens_out` — minimum tokens to receive (slippage protection)

**Flow:**
1. Read current reserves and LP supply
2. Compute proportional share: `sol_out = lp_amount * sol_reserve / lp_supply`, `tokens_out = lp_amount * token_reserve / lp_supply`
3. Slippage checks: `sol_out >= min_sol_out`, `tokens_out >= min_tokens_out`
4. Burn LP tokens from user
5. Transfer SOL from pool PDA to user
6. Transfer tokens from vault to user (transfer fee applies — user receives net)

**Constraints:**
- User must hold sufficient LP tokens
- Cannot remove all liquidity (minimum reserve enforced)

### 4. `swap`

Exchange SOL for tokens or tokens for SOL.

**Inputs:**
- `amount_in` — amount of input asset
- `minimum_out` — slippage protection
- `buy` — true = SOL to Token, false = Token to SOL

**Flow (buy — SOL to Token):**
1. Read current reserves
2. Apply 0.25% fee: `fee = amount_in * 25 / 10000`
3. `effective_in = amount_in - fee`
4. Compute output: `tokens_out = (effective_in * token_reserve) / (sol_reserve + effective_in)`
5. Slippage check: `tokens_out >= minimum_out`
6. Transfer SOL from user to pool PDA
7. Transfer tokens from vault to user (transfer fee applies)
8. Fee SOL stays in pool PDA — K increases

**Flow (sell — Token to SOL):**
1. Read current reserves
2. Transfer tokens from user to vault (measure net received after transfer fee)
3. Apply 0.25% fee on net received: `fee = net_tokens * 25 / 10000`
4. `effective_in = net_tokens - fee`
5. Compute output: `sol_out = (effective_in * sol_reserve) / (token_reserve + effective_in)`
6. Slippage check: `sol_out >= minimum_out`
7. Transfer SOL from pool PDA to user
8. Fee tokens stay in vault — K increases

**Invariant:** After every swap, `new_sol * new_tokens >= old_sol * old_tokens`. K never decreases.

## Accounts

### `Pool`

```
seeds = ["deep_pool", token_mint]
```

| Field | Type | Description |
|-------|------|-------------|
| token_mint | Pubkey | The Token-2022 mint |
| token_vault | Pubkey | Token vault ATA (owned by this PDA) |
| lp_mint | Pubkey | LP token mint (authority = this PDA) |
| initial_sol | u64 | SOL deposited at creation (immutable reference) |
| initial_tokens | u64 | Tokens deposited at creation (immutable reference) |
| total_swaps | u64 | Swap counter |
| bump | u8 | PDA bump |

SOL reserve is not stored — it's the pool PDA's lamport balance minus rent-exempt minimum. Always accurate, no tracking drift.

### Token Vault

Standard Token-2022 ATA owned by the Pool PDA.

```
owner = Pool PDA
mint = token_mint
token_program = Token-2022
```

### LP Mint

Token-2022 mint with mint authority = Pool PDA. No freeze authority. No transfer fee on LP tokens.

```
mint_authority = Pool PDA
freeze_authority = None
decimals = 6
```

## Constants

```
POOL_SEED       = "deep_pool"
SWAP_FEE_BPS    = 25        // 0.25%
FEE_DENOMINATOR = 10000
MIN_LIQUIDITY   = 1000      // minimum LP tokens locked on first deposit (prevents rounding attacks)
```

## Fee Model

Fees are applied on the input side before the constant product calculation:

```
fee = input * SWAP_FEE_BPS / FEE_DENOMINATOR
effective_input = input - fee
output = (effective_input * output_reserve) / (input_reserve + effective_input)
```

The fee stays in the pool as the input asset. This increases K after every swap. LP token holders benefit proportionally — their share of a growing pool appreciates without any action.

There is no fee recipient. No protocol treasury. The fee exists only to deepen the pool.

## LP Token Math

**Initial mint:** `lp_tokens = sqrt(sol_deposit * token_deposit) - MIN_LIQUIDITY`

The `MIN_LIQUIDITY` is permanently locked (minted to a burn address or the pool PDA itself) to prevent the first-depositor rounding attack where someone mints 1 LP token for a tiny deposit then donates to inflate the share price.

**Subsequent mints:** `lp_tokens = lp_supply * deposit_amount / reserve_amount`

Computed from whichever side of the deposit is the binding constraint (proportional deposit enforced).

**Redemption:** `sol_out = lp_amount * sol_reserve / lp_supply`, `tokens_out = lp_amount * token_reserve / lp_supply`

Rounding: floor on outputs (favors pool), ensures K never decreases on removal.

## Token-2022 Transfer Fee Handling

Torch tokens have a 0.04% transfer fee. On sells (token to SOL), the vault receives less than the user sends. The swap math must use the **net received** amount:

```
gross_sent = user's token amount
net_received = vault_balance_after - vault_balance_before
// Use net_received in swap math
```

On buys (SOL to token), the user receives less than the vault sends. This is the user's cost — the pool math uses the gross output from the vault.

Same pattern for `add_liquidity` — measure net tokens received, compute LP proportionally from net amount.

## Integration with Torch

### Migration

Torch's `migrate_to_dex` CPIs into DeepPool `create_pool`:
1. Deposit bonding curve SOL + remaining tokens
2. Receive LP tokens
3. Burn LP tokens (permanent liquidity)
4. No WSOL wrapping needed
5. No external authority to revoke

### Depth-Anchored Risk Model

Reads `pool_pda.lamports() - rent_exempt_minimum` for SOL reserve. Works directly with `get_depth_max_ltv_bps`. No changes to risk model.

### Fee Harvesting

`swap_fees_to_sol` CPIs into DeepPool `swap` instead of Raydium. Same flow, simpler instruction.

### Community LP

After Torch burns its LP at migration, community members can `add_liquidity` to further deepen the pool. They receive LP tokens they can hold or `remove_liquidity` later. Their LP appreciates as swap fees compound.

## Security Properties

1. **No pool admin** — no fee switch, no pause, no close. Program upgradeable but pool state is permanent.
2. **No extraction** — swap fees compound into pool. Protocol takes nothing.
3. **Burned LP is permanent** — Torch burns its LP, that liquidity is locked forever
4. **Community LP is redeemable** — add/remove is proportional, no lock
5. **K monotonic** — K only increases (fees + rounding direction)
6. **Atomic** — each swap is a single transaction
7. **Deterministic** — output is a pure function of reserves + input
8. **SOL reserve = lamports - rent** — no tracking drift
9. **First-depositor attack mitigated** — MIN_LIQUIDITY locked on creation

## Attack Surface

| Vector | Defense |
|--------|---------|
| First-depositor inflation | MIN_LIQUIDITY permanently locked |
| Pool creation spam | Costs rent + initial liquidity |
| Price manipulation | Constant product = quadratic slippage |
| Sandwich attacks | 0.25% fee makes sandwiching expensive |
| Token-2022 fee mismatch | Net vault balance measured, not input amount |
| Rounding exploits | Floor on output (favors pool) |
| LP drain | Proportional removal only — can't drain one side |
| Re-entrancy | Anchor CPI safety |
| Admin exploit | No pool admin exists |

## File Structure

```
programs/deep_pool/src/
  lib.rs              — entrypoint, 4 instructions
  state.rs            — Pool account struct
  constants.rs        — seeds, fee rate, min liquidity
  error.rs            — error codes
  instructions/
    create_pool.rs    — pool initialization + first LP mint
    add_liquidity.rs  — proportional deposit + LP mint
    remove_liquidity.rs — LP burn + proportional withdrawal
    swap.rs           — buy/sell with fee compounding
```
