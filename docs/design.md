# DeepPool: Immutable Constant-Product Pool Protocol

## Overview

DeepPool is a permissionless, constant-product AMM for Solana. Each pool pairs a Token-2022 token with native SOL — no WSOL wrapping. A 0.25% swap fee compounds 100% back into the pool — no protocol fees, no admin, no extraction. Once created, a pool cannot be modified or shut down.

Pools live in signer-verified namespaces: the creator signs for a `config` pubkey at creation time, and the pool's address is derived from `(config, token_mint)`. This prevents pool-squatting attacks and isolates protocol integrations into their own address space.

LP tokens are permanently locked on every deposit — 20% for pool creators, 7.5% for community LPs. The locked LP is minted to the pool PDA — an address that can never call `remove_liquidity`. Pools can only get deeper over time: swap fees compound, locked LP accumulates, and depth is a ratchet that never goes backwards.

Designed as the liquidity layer for Torch Market, replacing the Raydium dependency. Compatible with the depth-anchored risk model — pool SOL reserve is readable on-chain for margin risk gating.

## Constraint

Everything on-chain. No governance. No protocol fee. Pools are immutable on creation — no admin can modify, pause, or close an existing pool. The program itself is upgradeable (bug fixes, new features), but upgrades cannot affect existing pool state or assets.

## Design Principles

1. **Signer-verified namespaces** — every pool is keyed on a pubkey the creator must sign for, making pool-squatting cryptographically impossible
2. **Immutable pools** — no owner, no authority, no modification after creation
3. **Permissionless** — anyone can create a pool (in their own namespace), add liquidity, or swap
4. **Self-deepening** — 0.25% fee stays in the pool, K grows with every trade
5. **LP locks as reserve floor** — creators lock 20%, community LPs lock 7.5%. Permanent, ratchets upward.
6. **Token-2022 native** — no WSOL, no token wrapping, handles transfer fees natively
7. **Native SOL with self-authenticating transfers** — all SOL flow uses `System.transfer`, which the system program verifies against real lamport balances. No implicit-trust CPI paths.
8. **Token/SOL account separation** — swap takes a `user` (token authority) and a `sol_source` (SOL flow). CPI callers can route token ops and SOL through different PDAs.
9. **No freeze authority** — LP tokens can never be frozen by anyone
10. **Minimal** — four instructions, four account types, two invariants

## Namespace Model

Pool addresses are derived from three components:

```
pool_address = PDA(["deep_pool", config, token_mint], program_id)
```

The `config` must sign `create_pool`. Two usage patterns:

**Protocol namespace.** Config is a program PDA, signed via `CpiContext::new_with_signer`. Example: torch uses `PDA(["torch_config"], torch_program_id)`. Only torch can create in torch's namespace.

**Wallet namespace.** Config is the creator's wallet pubkey. They sign the transaction directly. Their namespace is their wallet.

Downstream instructions (`swap`, `add_liquidity`, `remove_liquidity`) don't require the config to sign — they re-derive the pool PDA from the stored `pool.config` at load time and fail if substituted. The signer requirement is only at creation.

Pools in different namespaces are at different addresses. They can't interfere, can't collide, can't be squatted. This closes the pool-initialization-griefing vulnerability class documented in [pool-initialization-griefing.md](./pool-initialization-griefing.md).

## Instructions

### 1. `create_pool`

Initialize a new pool for a Token-2022 mint paired with native SOL, under a namespace the caller signs for.

**Signers:**
- `creator` — pays rent, deposits initial liquidity
- `config` — namespace authority (may equal creator for wallet-created pools)

**Inputs:**
- `initial_token_amount` — tokens to deposit
- `initial_sol_amount` — SOL lamports to deposit

**Flow:**
1. Derive pool PDA from `(config, token_mint)`
2. Create pool state, token vault, LP mint (no freeze authority)
3. Transfer tokens from creator to vault via `transfer_checked`, measure net received (handles Token-2022 transfer fees)
4. `System.transfer` SOL from creator to pool PDA
5. Compute initial LP supply: `sqrt(sol_amount * net_tokens) - MIN_LIQUIDITY`
6. Mint 80% LP to creator, 20% LP to `pool_lp_account` (permanently locked — the pool PDA cannot sign `remove_liquidity`)
7. Record initial reserves, namespace config, and bumps. Pool is live.

**Constraints:**
- One pool per `(config, mint)` pair — PDA enforced
- `initial_sol_amount >= MIN_INITIAL_SOL` (0.1 SOL)
- `initial_token_amount >= MIN_INITIAL_TOKENS` (1 token)
- Token must be Token-2022 (owner check)
- `sqrt(sol * tokens) > MIN_LIQUIDITY` (floor against first-depositor inflation)

### 2. `add_liquidity`

Deposit SOL + tokens proportionally, receive LP tokens.

**Inputs:**
- `token_amount` — tokens to deposit
- `max_sol_amount` — maximum SOL willing to deposit (slippage protection)

**Flow:**
1. Compute required SOL: `sol_required = token_amount * sol_reserve / token_reserve`
2. Slippage check: `sol_required <= max_sol_amount`
3. `transfer_checked` tokens to vault, measure net received
4. `System.transfer` SOL to pool PDA
5. Compute LP: `lp_amount = lp_supply * net_tokens / token_reserve`
6. Mint 92.5% LP to provider, 7.5% LP to `pool_lp_account` (locked)

**Constraints:**
- Pool must exist with non-zero reserves and non-zero LP supply
- Proportional deposit enforced — no single-sided adds

### 3. `remove_liquidity`

Burn LP tokens, receive proportional share of SOL + tokens.

**Inputs:**
- `lp_amount` — LP tokens to burn
- `min_sol_out` — minimum SOL to receive (slippage)
- `min_tokens_out` — minimum tokens to receive (slippage)

**Flow:**
1. Compute proportional share: `sol_out = lp_amount * sol_reserve / lp_supply`, same for tokens
2. Slippage checks
3. Ensure pool retains `sol_remaining > 0 && tokens_remaining > 0` (non-negative invariant; the LP lock math implicitly enforces a much stronger floor — see "LP Lock" below)
4. Burn LP from provider
5. `transfer_checked` tokens from vault to provider
6. Direct lamport credit from pool PDA to provider

**Key behavior:** Because the pool PDA holds locked LP from every deposit, `lp_supply` is larger than the sum of all user-held LP. The fraction any set of users can collectively redeem is strictly less than 1, and the lock ratio ratchets upward over time.

### 4. `swap`

Exchange SOL for tokens (buy) or tokens for SOL (sell). The same instruction handles both directions via a `buy: bool` flag.

**Signers:**
- `user` — token authority; owns `user_token_account` (ATA)
- `sol_source` — SOL source on buy / SOL sink on sell; must be system-owned on buy (for `System.transfer`)

For wallet callers, `sol_source == user` (one wallet signs both). For CPI callers, `sol_source` is typically a distinct system-owned PDA the caller signs for — see "Integration with Torch" below.

**Inputs:**
- `amount_in` — amount of input asset
- `minimum_out` — slippage protection
- `buy` — true = SOL→Token, false = Token→SOL

**Flow (buy, SOL→Token):**
1. Read `sol_reserve = pool.lamports - rent_exempt` and `token_reserve = token_vault.amount`
2. Fee: `fee = amount_in * 25 / 10000`, `effective_in = amount_in - fee`
3. Output: `tokens_out = effective_in * token_reserve / (sol_reserve + effective_in)` (u128 intermediate)
4. Slippage check: `tokens_out >= minimum_out`
5. `System.transfer(from=sol_source, to=pool, amount_in)` — self-authenticated by the system program
6. `transfer_checked` tokens from vault to `user_token_account` (pool PDA signs)

**Flow (sell, Token→SOL):**
1. Read reserves
2. `transfer_checked` tokens from `user_token_account` to vault, measure net received (handles Token-2022 transfer fees)
3. Fee on net received: `fee = net * 25 / 10000`
4. Output: `sol_out = effective_in * sol_reserve / (token_reserve + effective_in)`
5. Slippage check
6. Direct lamport credit: `pool.lamports -= sol_out; sol_source.lamports += sol_out`

**Invariant:** `K_new >= K_old` after every swap. K never decreases. Proven by Kani for concrete cases, proptest-verified across 10,000 random cases per property.

**Why `sol_source` is separate from `user`:** The buy path does `System.transfer(from=sol_source)`, which requires `sol_source.owner == system_program`. A CPI caller's state PDA is program-owned and can't be `from`. Splitting `user` (token authority, may be program-owned) and `sol_source` (SOL flow, must be system-owned for buys) lets CPI callers like torch use a program-owned state PDA for token ATAs while routing SOL through a separate system-owned lamport-holder PDA. Wallet callers pass the same account twice; Solana deduplicates the signature.

## Reserve Floor via LP Lock

The LP lock isn't just "some LP is unredeemable" — it's the **reserve floor mechanism**. No explicit `require!(sol_remaining >= X)` check is needed because the math enforces the floor automatically.

At any point in time:
```
reserve_floor = (locked_LP / total_LP_supply) × current_reserve
```

Because:
- `locked_LP` sits in `pool_lp_account`, owned by the pool PDA
- The pool PDA cannot sign `remove_liquidity` in any instruction
- Therefore `locked_LP` never decreases
- Redeemable LP = `total_LP_supply - locked_LP`
- Max fraction of reserve that can be withdrawn = `(total - locked) / total`
- Remaining = `locked / total` × reserve

**The floor ratchets upward with every `add_liquidity`:**
- 7.5% of each new deposit joins the locked LP
- The locked fraction grows monotonically with deposit history
- Repeated add/remove cycles by the same LP compound: `0.925^n` of original value retained after n cycles

**For a minimum-size pool (0.1 SOL initial):**
- Creator contributes 0.1 SOL, receives 80% of LP
- 20% of LP is locked at `pool_lp_account`
- Maximum possible drain: creator redeems all their LP → pool retains 0.02 SOL + rent
- This is the floor

## Constants

```
POOL_SEED            = "deep_pool"
VAULT_SEED           = "pool_vault"
LP_MINT_SEED         = "pool_lp_mint"
SWAP_FEE_BPS         = 25          // 0.25%
FEE_DENOMINATOR      = 10000
LP_LOCK_CREATOR_BPS  = 2000        // 20% locked on create_pool
LP_LOCK_PROVIDER_BPS = 750         // 7.5% locked on add_liquidity
MIN_LIQUIDITY        = 1000        // subtracted from initial sqrt — anti first-depositor-inflation floor
MIN_INITIAL_SOL      = 100_000_000 // 0.1 SOL
MIN_INITIAL_TOKENS   = 1_000_000   // 1 token (6 decimals)
```

## LP Lock Economics

The split lock rates create aligned incentives:

- **Creators have skin in the game** — 20% lock means they're committed to the pool they created
- **Community LPs get a better deal** — 7.5% is low enough not to scare away liquidity
- **Promote, don't exit** — the best way to increase LP value is to drive swap volume, since fees compound into K
- **Reserve floor** — locked LP is the mechanism; floor = locked_fraction × reserve and ratchets up with every deposit
- **Early LPs rewarded** — as the pool deepens, their share appreciates from both fees and subsequent providers locking their portion

## Token-2022 Transfer Fee Handling

Torch tokens have a 0.04% transfer fee. On sells and liquidity adds, the vault receives less than the user sends. Swap math uses **net received**:

```
net_received = vault_balance_after - vault_balance_before
```

Same for `add_liquidity` — LP computed from net amount. Same for `create_pool` — initial LP computed from net tokens received.

## Integration with Torch

Torch Market uses DeepPool as its post-migration liquidity layer. Integration points:

### Namespace
Torch defines `torch_config = PDA(["torch_config"], torch_program_id)`. All migrated pools live at `PDA(["deep_pool", torch_config, mint], deep_pool_program_id)`. Only the torch program can sign for `torch_config` via `invoke_signed`, so no one else can create in torch's namespace.

### Migration
When a bonding curve completes, torch CPIs into `create_pool` with:
- `creator` = torch's treasury PDA
- `config` = `torch_config` (signed via CPI)
- Initial SOL + tokens from the bonding curve

LP minted to torch. Torch may burn it to lock liquidity permanently, layered on top of DeepPool's 20% creator lock.

### Vault swap (user trading via torch)
Torch's `vault_swap` instruction CPIs into DeepPool's `swap` with:
- `user` = `torch_vault` (program-owned PDA holding the user's token ATA)
- `sol_source` = `vault_sol` (system-owned sibling PDA, 0 lamports between swaps)

**On buy:** torch moves `amount_in` lamports from `torch_vault` (state) to `vault_sol` (system-owned) via direct lamport manipulation. Calls swap. DeepPool's `System.transfer(from=vault_sol)` pulls the lamports. Tokens land in `ATA(torch_vault)`.

**On sell:** torch passes `sol_source = torch_vault` directly. DeepPool credits lamports via direct manipulation, which is owner-agnostic. No `vault_sol` traffic.

### Fee harvesting
Torch's `swap_fees_to_sol` CPIs into DeepPool `swap` (sell-only) to convert harvested Token-2022 transfer fees into SOL for the treasury. Uses `sol_source = treasury`.

### Depth model
Torch's risk engine reads `pool_pda.lamports() - rent` for the live SOL reserve. Used for margin gating on shorts and loans.

## File Structure

```
programs/deep_pool/src/
  lib.rs                    — entrypoint, 4 instructions
  state.rs                  — Pool account struct + sol_reserve helper
  constants.rs              — seeds, fee rate, LP lock rates, minimums, TOKEN_2022_PROGRAM_ID
  error.rs                  — error codes
  math.rs                   — checked constant-product math + integer_sqrt
  kani_proofs.rs            — 16 formal verification proofs
  instructions.rs           — module root
  instructions/
    create_pool.rs          — pool init, signer-verified namespace, initial LP mint (80/20)
    add_liquidity.rs        — proportional deposit, LP mint (92.5/7.5)
    remove_liquidity.rs     — LP burn, proportional withdrawal
    swap.rs                 — buy/sell, unified System.transfer path, sol_source split
programs/deep_pool/tests/
  math_proptests.rs         — 16 property-based fuzz tests (10,000 cases each)
```
