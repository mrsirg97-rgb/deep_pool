# Pool Namespacing: Signer-Verified Config Seeds

## The Vulnerability

Every constant-product AMM with deterministic pool addresses is vulnerable to pool initialization DOS. The attack:

1. Attacker monitors mempool for pool creation transactions
2. Attacker frontruns with the same token pair but malicious parameters (garbage ratio, far-future activation time, dust liquidity)
3. The pool PDA is claimed — no one else can create a pool for that token
4. The token is permanently bricked on that DEX

This affects any protocol where `pool_pda = f(token_pair)` and only one pool per pair exists. Documented by Fuzzing Labs against Raydium CLMM (January 2024). The vulnerability is not chain-specific.

### Existing "Fixes"

| Approach | Problem |
|----------|---------|
| Parameter bounds checking (Raydium) | Patches one symptom. PDA squatting still possible. |
| Admin-only pool creation | Centralized. Defeats permissionless design. |
| Multiple pools per pair (Uniswap V4) | Fragments liquidity. Requires routing logic. Complex. |
| Hope nobody does it | Not a fix. |

## The Fix: Signer-Verified Config Seeds

### Concept

Add a **config** account to the pool PDA derivation. The config must be a **signer** on the `create_pool` transaction. The pool PDA becomes:

```
seeds = ["deep_pool", config.key(), token_mint.key()]
```

The config is not an on-chain account with state — it's just a pubkey used as a namespace seed. But because it must sign, no one can use someone else's config.

### How It Works

**Torch migration (CPI):**
- Config = Torch program's PDA (e.g., `["torch_config"]`)
- Torch signs for this PDA via CPI (`CpiContext::new_with_signer`)
- Pool PDA = `["deep_pool", torch_config_pda, mint]`
- Nobody else can create a pool under Torch's namespace because they can't sign for Torch's PDA

**Standalone pool creation (wallet):**
- Config = creator's wallet pubkey
- Creator signs normally (they're already the signer)
- Pool PDA = `["deep_pool", wallet, mint]`
- Nobody else can create a pool under that wallet's namespace

**Other protocol (CPI):**
- Config = that protocol's PDA
- Protocol signs via CPI
- Pool PDA = `["deep_pool", protocol_pda, mint]`
- Isolated from all other protocols

### Properties

1. **No squatting** — you can't sign for a key you don't control
2. **Permissionless** — anyone can create pools under their own namespace
3. **No fragmentation** — each protocol has exactly one canonical pool per token
4. **No centralization** — no admin, no governance, no whitelist
5. **Deterministic** — given the config + mint, the pool address is known
6. **Zero cost** — just an additional seed, no new accounts or state
7. **Backward compatible** — existing pools use `["deep_pool", mint]`, new pools use `["deep_pool", config, mint]`

### Why Signing Matters

Without signing, anyone can pass `TORCH_PROGRAM_ID` as the config bytes and squat Torch's namespace. The signer requirement means:

- Wallet configs: the wallet is already the transaction signer
- Program configs: only the program can produce a valid PDA signature via CPI
- No one can impersonate another signer

### Account Structure

```rust
#[derive(Accounts)]
pub struct CreatePool<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The namespace config. Must sign. Determines which pool PDA is created.
    /// For CPI callers: a program PDA signed via CpiContext::new_with_signer.
    /// For wallet callers: can be the creator (same signer).
    pub config: Signer<'info>,

    #[account(
        init,
        payer = creator,
        space = Pool::LEN,
        seeds = [POOL_SEED, config.key().as_ref(), token_mint.key().as_ref()],
        bump,
    )]
    pub pool: Account<'info, Pool>,
    // ... rest unchanged
}
```

### Downstream Instructions

Swap, add_liquidity, and remove_liquidity validate the pool PDA using seeds stored in the Pool state:

```rust
#[account(
    mut,
    seeds = [POOL_SEED, pool.config.as_ref(), pool.token_mint.as_ref()],
    bump = pool.bump,
)]
pub pool: Account<'info, Pool>,
```

The `config` is stored in the Pool account at creation. Downstream callers don't need to pass it — it's read from the pool data. Only `create_pool` requires the config signer.

### Pool State

```rust
pub struct Pool {
    pub config: Pubkey,      // namespace config (new field)
    pub token_mint: Pubkey,
    pub token_vault: Pubkey,
    pub lp_mint: Pubkey,
    pub initial_sol: u64,
    pub initial_tokens: u64,
    pub bump: u8,
}
```

### SDK Changes

```typescript
// Torch migration — config is Torch's PDA
const [torchConfig] = PublicKey.findProgramAddressSync(
  [Buffer.from("torch_config")],
  TORCH_PROGRAM_ID,
)
const [pool] = getPoolPda(torchConfig, tokenMint)

// Standalone — config is creator wallet
const [pool] = getPoolPda(creatorWallet, tokenMint)

// Reading — pool address is deterministic from config + mint
const pool = await getPool(connection, tokenMint, config)
```

### Torch Integration

Torch defines a config PDA: `seeds = ["torch_config"]` under the Torch program. During migration:

1. Torch derives `torch_config` PDA
2. Torch CPIs into DeepPool `create_pool` with `torch_config` as the config signer
3. Torch signs for `torch_config` via `CpiContext::new_with_signer`
4. Pool PDA = `["deep_pool", torch_config, mint]` — deterministic, unfrontrunnable

The torchsdk hardcodes `torch_config` derivation, same as it hardcodes `TORCH_PROGRAM_ID`. All downstream operations (vault swap, fee harvest, lending reads) derive the pool from `torch_config + mint`. No lookup needed.

### Attack Scenarios

**Griefer tries to squat Torch's namespace:**
- Needs to sign as `torch_config` PDA
- Can't — only the Torch program can produce that signature
- Attack fails

**Griefer creates pool under their own namespace:**
- Succeeds — creates `["deep_pool", griefer_wallet, mint]`
- Pool is isolated from Torch's namespace
- Nobody uses it, griefer loses 20% LP lock
- Torch migration creates `["deep_pool", torch_config, mint]` normally

**Griefer frontruns with same config + different params:**
- Impossible — same config + same mint = same PDA = `init` fails
- But the griefer can't get to that PDA first because they can't sign as the config

### Comparison

| | Old (single seed) | Multi-pool (creator seed) | Config signer (this proposal) |
|-|-------------------|--------------------------|-------------------------------|
| Frontrun resistant | No | Yes | Yes |
| Permissionless | Yes | Yes | Yes |
| Deterministic | Yes | No (need creator) | Yes (need config) |
| Fragmentation | N/A | Yes (multiple pools) | No (one per namespace) |
| SDK complexity | Simple | Complex (pool discovery) | Simple (config is constant) |
| Liquidity | Consolidated | Fragmented | Consolidated per namespace |
| Signing cost | None | None | None (already signing) |
