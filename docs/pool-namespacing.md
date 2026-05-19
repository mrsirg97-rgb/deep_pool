# Pool Namespacing: Signer-Verified Config Seeds

*A systemic vulnerability in permissionless DEX migration and the fix.*

## Abstract

We identify a class of griefing vulnerabilities affecting every decentralized exchange with deterministic pool addresses and every launchpad that migrates token liquidity to such exchanges. The attack requires no direct profit motive but causes severe user and protocol harm: anyone who holds tokens from a bonding curve can permanently prevent that token's liquidity migration by pre-creating the pool with malicious parameters. We document the attack, analyze its impact across major protocols, and propose a mitigation — signer-verified config seeds — that eliminates the vulnerability while preserving permissionless pool creation.

**Severity:** Medium — griefing attack (no direct profit motive, but persistent user/protocol harm).

## 1. Background

### 1.1 Bonding curves and DEX migration

Modern token launchpads (Pump.fun, Raydium LaunchLab, Torch Market) use bonding curves for initial price discovery. When sufficient liquidity accumulates, the token "graduates" and migrates to a constant-product AMM (Raydium, PumpSwap, DeepPool) for open trading. This migration creates a liquidity pool, deposits SOL and tokens, and burns LP tokens to lock liquidity permanently.

### 1.2 Deterministic pool addresses

AMMs on Solana derive pool addresses from Program Derived Addresses (PDAs). The address is a deterministic function of the token pair:

```
pool_address = PDA(["pool_seed", token_mint])             // DeepPool v1
pool_address = PDA(["pool", amm_config, token0, token1])  // Raydium CPMM
```

Because the address is deterministic, only one pool can exist per token (or per token pair per config). If the PDA is already allocated, `init` fails.

### 1.3 Permissionless pool creation

These DEXs are permissionless — anyone can create a pool for any token. This is a feature: it enables organic liquidity provision without admin gatekeeping. But it creates a conflict with migration: if anyone can create the pool, anyone can create it *before* the intended migration.

## 2. The griefing attack

### 2.1 Mechanism

1. Attacker buys tokens on the bonding curve (at any point — even 1% bonded)
2. Attacker now holds tokens in their wallet
3. Attacker calls `create_pool` on the target DEX with the token mint + some SOL
4. The pool PDA is now allocated with the attacker's chosen parameters (garbage ratio, dust liquidity)
5. When the bonding curve completes and migration fires, `create_pool` CPI fails: "account already in use"
6. The token is permanently bricked — it can never trade on that DEX

### 2.2 Cost to attacker

- Buy tokens on bonding: ~0.1–1 SOL
- Create pool: ~0.1 SOL (minimum initial deposit) + rent (~0.002 SOL)
- Total: **<2 SOL to permanently brick a token**

### 2.3 No direct profit motive — pure griefing

The attacker gains nothing financially. This is textbook griefing: **no direct profit, but severe user and protocol harm.** Motivations include:

- Competitor sabotage (brick rival tokens)
- Market manipulation (prevent graduation, trap traders in bonding curve)
- Extortion ("pay me or I brick your token")
- Short-selling related attacks (brick migration → token value collapses)

**User harm:** holders permanently trapped in the bonding curve with no exit to DEX liquidity.

**Protocol harm:** the launchpad's core value proposition (graduation to DEX) breaks. Reputation damage, user loss, potential legal liability.

### 2.4 Scalability

A single script monitoring the mempool for bonding-curve purchases can automatically:

1. Buy minimum tokens on any new bonding curve
2. Immediately create a pool on the target DEX
3. Brick every token on the platform

Cost: ~2 SOL per token. At scale, this could disable an entire launchpad.

## 3. Affected protocols

### 3.1 Raydium CPMM

Pool PDA: `["pool", amm_config, token0, token1]`. Any protocol migrating to Raydium CPMM is vulnerable. The `amm_config` is a known constant — the attacker uses the same config the protocol would use.

**Distinction from prior work:** Fuzzing Labs documented `open_time` manipulation on Raydium CLMM (concentrated liquidity) in January 2024. That vulnerability allowed manipulation of pool parameters. This disclosure covers a different attack surface: **PDA squatting via bonding-curve token acquisition on CPMM**, which permanently prevents migration rather than manipulating pool behavior. The CLMM patch (parameter bounds checking) does not mitigate CPMM PDA squatting.

### 3.2 Pump.fun → Raydium

Pump.fun migrated tokens to Raydium until March 2025. Tokens have been documented as "stuck migrating" on the platform. While the May 2024 incident was attributed to a rogue employee, the migration failure pattern matches PDA squatting. Pump.fun's response: launched PumpSwap, their own DEX, in March 2025. PumpSwap uses an `index` parameter in pool seeds to allow multiple pools per pair. Canonical migration uses `index = 0`. However, the index is not signer-verified — anyone can create `index = 0` before migration.

### 3.3 Raydium LaunchLab

Raydium's own launchpad, launched in response to Pump.fun. Migrates to Raydium AMM pools. Same deterministic PDA, same vulnerability.

### 3.4 Any future launchpad

Any protocol that (1) sells tokens via bonding curve (or any mechanism that puts tokens in user wallets) and (2) later migrates liquidity to a DEX with deterministic pool addresses is vulnerable. This includes potential future protocols on Ethereum (Uniswap V4 has deterministic pool addresses), Base, Arbitrum, or any EVM chain.

## 4. Existing mitigations (and why they're insufficient)

| Approach | Problem |
|----------|---------|
| Parameter bounds checking (Raydium CLMM) | Patches one symptom (`open_time` manipulation). PDA squatting still possible. |
| Admin-only pool creation | Centralized. Defeats permissionless design. |
| Index-based multiple pools (PumpSwap) | Index not cryptographically verified — anyone can race for `index = 0`. Liquidity fragments across indices; routing complexity. |
| Multiple pools per pair (Uniswap V4) | Fragments liquidity. Requires routing logic. Complex. |
| Building your own DEX (Pump.fun) | Works, but centralized — requires vertical integration, prevents composability with other DEXs. |
| Hope nobody does it | Not a fix. |

## 5. Proposed solution: signer-verified config seeds

### 5.1 Design

Add a **config** account to the pool PDA derivation. The config must be a **signer** on the `create_pool` transaction:

```
pool_address = PDA(["pool_seed", config.key(), token_mint.key()])
```

The config is not an on-chain state account — it's a pubkey used as a namespace seed. Because it must sign, no one can use someone else's config.

### 5.2 Usage modes

**Protocol migration (CPI):**
- Config = protocol's PDA (e.g., `PDA(["torch_config"], TORCH_PROGRAM_ID)`)
- Protocol signs via `CpiContext::new_with_signer`
- Nobody else can produce this signature → nobody else can squat the protocol's namespace

**Standalone pool creation (wallet):**
- Config = creator's wallet pubkey
- Creator is already signing the transaction
- Their namespace is their wallet — unique by default

**Cross-protocol composability:**
- Each protocol has its own namespace
- Pools are isolated by config
- No collisions, no squatting, no coordination needed

### 5.3 Properties

| Property | Status |
|---|---|
| Frontrun resistant | Yes — can't sign for someone else's config |
| Permissionless | Yes — anyone can create pools in their namespace |
| Deterministic | Yes — PDA from `(config, mint)` is unique and known |
| No fragmentation | Each namespace has exactly one pool per token |
| No centralization | No admin, no governance, no whitelist |
| Zero additional cost | One extra seed in PDA derivation |
| Backward compatible | Existing pools unaffected by upgrade |

### 5.4 Why signing matters

Without signing, anyone can pass any pubkey as config bytes — including the target protocol's program ID. The signer requirement provides cryptographic proof of namespace ownership:

- **Wallet configs:** the wallet signs the transaction (standard Solana signing).
- **Program configs:** only the program can produce a valid PDA signature via CPI.
- **No impersonation possible:** Solana's signature verification is at the runtime level.

### 5.5 Comparison vs. alternatives

| | Old (single seed) | Multi-pool (creator seed) | **Config signer (this proposal)** |
|-|-------------------|--------------------------|-------------------------------|
| Frontrun resistant | No | Yes | Yes |
| Permissionless | Yes | Yes | Yes |
| Deterministic | Yes | No (need creator) | Yes (need config) |
| Fragmentation | N/A | Yes (multiple pools) | No (one per namespace) |
| SDK complexity | Simple | Complex (pool discovery) | Simple (config is constant) |
| Liquidity | Consolidated | Fragmented | Consolidated per namespace |
| Signing cost | None | None | None (already signing) |

## 6. Implementation (DeepPool v2.0+)

### 6.1 Account structure

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
    // ... rest of accounts
}
```

### 6.2 Pool state

```rust
pub struct Pool {
    pub config: Pubkey,      // namespace config — stored at creation
    pub token_mint: Pubkey,
    pub token_vault: Pubkey,
    pub lp_mint: Pubkey,
    pub initial_sol: u64,
    pub initial_tokens: u64,
    pub bump: u8,
}
```

### 6.3 Downstream instructions

Only `create_pool` requires the config signer. Swap, `add_liquidity`, and `remove_liquidity` read the config from pool state and use it for PDA validation:

```rust
#[account(
    mut,
    seeds = [POOL_SEED, pool.config.as_ref(), pool.token_mint.as_ref()],
    bump = pool.bump,
)]
pub pool: Account<'info, Pool>,
```

No additional signing required for trading or LP operations.

### 6.4 SDK shape

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

The SDK hardcodes per-protocol config derivation the same way it hardcodes program IDs. All downstream operations (vault swap, fee harvest, lending reads) derive the pool from `(config, mint)`. No on-chain lookup needed.

### 6.5 Torch Market integration

Torch defines a config PDA: `seeds = ["torch_config"]` under the Torch program. During migration:

1. Torch derives `torch_config` PDA
2. Torch CPIs into DeepPool `create_pool` with `torch_config` as the config signer
3. Torch signs for `torch_config` via `CpiContext::new_with_signer`
4. Pool PDA = `["deep_pool", torch_config, mint]` — deterministic, unfrontrunnable

### 6.6 Attack scenarios under the fix

**Griefer tries to squat Torch's namespace:**
- Needs to sign as `torch_config` PDA.
- Can't — only the Torch program can produce that signature.
- Attack fails.

**Griefer creates a pool under their own namespace:**
- Succeeds — creates `["deep_pool", griefer_wallet, mint]`.
- Pool is isolated from Torch's namespace.
- Nobody uses it; griefer loses 20% LP lock.
- Torch migration creates `["deep_pool", torch_config, mint]` normally.

**Griefer frontruns with same config + different params:**
- Impossible — same config + same mint = same PDA = `init` fails.
- And the griefer can't get to that PDA first because they can't sign as the config.

## 7. Responsible disclosure

This paper documents a griefing vulnerability class affecting multiple production protocols. The specific attack vector (PDA squatting via bonding-curve token purchase + premature pool creation on CPMM) has not been publicly documented. Prior work by Fuzzing Labs (January 2024) identified `open_time` manipulation on Raydium CLMM — a different program, different attack surface, and different impact (parameter manipulation vs. permanent migration failure).

Affected protocols should consider:

1. Implementing signer-verified config seeds (recommended)
2. Adding CPI origin checks to pool creation
3. Implementing fallback migration paths
4. Monitoring for anomalous pool creation patterns

## 8. Conclusion

Pool initialization griefing is a systemic vulnerability in permissionless DeFi. The attack requires no direct profit motive but causes severe, permanent harm to users (funds trapped) and protocols (migration broken). Every launchpad that migrates to a deterministic-address DEX is affected. Existing mitigations are either centralized (admin-only creation), incomplete (parameter bounds checking), or complex (index-based multiple pools).

Signer-verified config seeds eliminate the vulnerability at the protocol level while preserving permissionless design. The fix is minimal (one additional PDA seed + signer check), composable (any protocol can use it), and cryptographically enforced (no spoofing possible).

The reference implementation is open source: [github.com/mrsirg97-rgb/deep_pool](https://github.com/mrsirg97-rgb/deep_pool).

## References

- Fuzzing Labs. "DOS — DeFi Liquidity Pools: The Initialization Vulnerability." January 2024. https://fuzzinglabs.com/raydium-dos-initialization/
- Raydium. CLMM Bug Bounty Details. https://docs.raydium.io/raydium/protocol/bug-bounty-program/clmm-bug-bounty-details
- Pump.fun. "Coin Migration Issue Post-Mortem." May 2024. https://x.com/pumpdotfun/status/1791235050643636303
- PumpSwap Program Documentation. https://deepwiki.com/pump-fun/pump-public-docs/4-pumpswap-program
- Immunefi. "Raydium Tick Manipulation Bugfix Review." 2024. https://immunefi.com/blog/bug-fix-reviews/raydium-tick-manipulation-bugfix-review/

## Authors

Built by the Torch Market team.

**DeepPool reference implementation:**

- Source: [github.com/mrsirg97-rgb/deep_pool](https://github.com/mrsirg97-rgb/deep_pool)
- Program ID: `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`
