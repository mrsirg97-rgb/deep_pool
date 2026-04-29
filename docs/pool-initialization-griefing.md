# Pool Initialization Griefing: A Systemic Vulnerability in Permissionless DEX Migration

## Abstract

We identify a class of griefing vulnerabilities affecting every decentralized exchange with deterministic pool addresses and every launchpad that migrates token liquidity to such exchanges. The attack requires no direct profit motive but causes severe user and protocol harm: anyone who holds tokens from a bonding curve can permanently prevent that token's liquidity migration by pre-creating the pool with malicious parameters. We document the attack, analyze its impact across major protocols, and propose a novel mitigation — signer-verified config seeds — that eliminates the vulnerability while preserving permissionless pool creation.

**Severity:** Medium — Griefing attack (no direct profit motive, but user or protocol harm)

## 1. Background

### 1.1 Bonding Curves and DEX Migration

Modern token launchpads (Pump.fun, Raydium LaunchLab, Torch Market) use bonding curves for initial price discovery. When sufficient liquidity accumulates, the token "graduates" and migrates to a constant-product AMM (Raydium, PumpSwap, DeepPool) for open trading. This migration creates a liquidity pool, deposits SOL and tokens, and burns LP tokens to lock liquidity permanently.

### 1.2 Deterministic Pool Addresses

AMMs on Solana derive pool addresses from Program Derived Addresses (PDAs). The address is a deterministic function of the token pair:

```
pool_address = PDA(["pool_seed", token_mint])           // DeepPool
pool_address = PDA(["pool", amm_config, token0, token1]) // Raydium CPMM
```

Because the address is deterministic, only one pool can exist per token (or per token pair per config). If the PDA is already allocated, `init` fails.

### 1.3 Permissionless Pool Creation

These DEXs are permissionless — anyone can create a pool for any token. This is a feature: it enables organic liquidity provision without admin gatekeeping. But it creates a conflict with migration: if anyone can create the pool, anyone can create it *before* the intended migration.

## 2. The Griefing Attack

### 2.1 Mechanism

1. Attacker buys tokens on the bonding curve (at any point — even 1% bonded)
2. Attacker now holds tokens in their wallet
3. Attacker calls `create_pool` on the target DEX with the token mint + some SOL
4. The pool PDA is now allocated with the attacker's chosen parameters (garbage ratio, dust liquidity)
5. When the bonding curve completes and migration fires, `create_pool` CPI fails: "account already in use"
6. The token is permanently bricked — it can never trade on that DEX

### 2.2 Cost to Attacker

- Buy tokens on bonding: ~0.1-1 SOL (can buy minimum amount)
- Create pool: ~0.1 SOL (minimum initial deposit) + rent (~0.002 SOL)
- Total: <2 SOL to permanently brick a token

### 2.3 No Direct Profit Motive — Pure Griefing

The attacker gains nothing financially. This is a textbook griefing attack: **no direct profit motive, but severe user and protocol harm.** Potential motivations include:
- Competitor sabotage (brick rival tokens)
- Market manipulation (prevent graduation, trap traders in bonding curve)
- Extortion ("pay me or I brick your token")
- Short-selling related attacks (brick migration → token value collapses)

**User harm:** Token holders are permanently trapped in the bonding curve with no exit to DEX liquidity.

**Protocol harm:** The launchpad's core value proposition (graduation to DEX) is broken. Reputation damage, user loss, potential legal liability.

### 2.4 Scalability

A single script monitoring the mempool for bonding curve purchases can automatically:
1. Buy minimum tokens on any new bonding curve
2. Immediately create a pool on the target DEX
3. Brick every token on the platform

Cost: ~2 SOL per token. At scale, this could disable an entire launchpad.

## 3. Affected Protocols

### 3.1 Raydium CPMM

Pool PDA: `["pool", amm_config, token0, token1]`

Any protocol migrating to Raydium CPMM is vulnerable. The `amm_config` is a known constant — the attacker uses the same config the protocol would use.

**Distinction from prior work:** Fuzzing Labs documented `open_time` manipulation on Raydium CLMM (concentrated liquidity) in January 2024. That vulnerability allowed manipulation of pool parameters. This disclosure covers a different attack surface: **PDA squatting via bonding curve token acquisition on CPMM**, which permanently prevents migration rather than manipulating pool behavior. The CLMM patch (parameter bounds checking) does not mitigate CPMM PDA squatting.

### 3.2 Pump.fun → Raydium

Pump.fun migrated tokens to Raydium until March 2025. Tokens have been documented as "stuck migrating" on the platform. While the May 2024 incident was attributed to a rogue employee, the migration failure pattern matches PDA squatting.

Pump.fun's response: launched PumpSwap, their own DEX, in March 2025. PumpSwap uses an `index` parameter in pool seeds to allow multiple pools per pair. Canonical migration uses `index = 0`. However, the index is not signer-verified — anyone can create `index = 0` before migration.

### 3.3 Raydium LaunchLab

Raydium's own launchpad, launched in response to Pump.fun. Migrates to Raydium AMM pools. Same deterministic PDA, same vulnerability.

### 3.4 Any Future Launchpad

Any protocol that:
1. Sells tokens via bonding curve (or any mechanism that puts tokens in user wallets)
2. Later migrates liquidity to a DEX with deterministic pool addresses

...is vulnerable. This includes potential future protocols on Ethereum (Uniswap V4 has deterministic pool addresses), Base, Arbitrum, or any EVM chain.

## 4. Existing Mitigations (and Why They're Insufficient)

### 4.1 Parameter Bounds Checking (Raydium)

Raydium patched the `open_time` manipulation by adding bounds checking. This prevents one specific attack vector but does not prevent PDA squatting. An attacker can create a pool with valid parameters but a garbage price ratio.

### 4.2 Admin-Only Pool Creation

Some protocols restrict pool creation to an admin address. This eliminates squatting but sacrifices permissionless design — the core value proposition of DeFi.

### 4.3 Index-Based Multiple Pools (PumpSwap)

PumpSwap adds an `index` parameter to pool seeds, allowing multiple pools per pair. `index = 0` is reserved for canonical migration. This is better than single-pool but:
- The index is not cryptographically verified
- Anyone can race for `index = 0`
- Liquidity fragments across indices
- Routing complexity increases

### 4.4 Building Your Own DEX (Pump.fun)

Pump.fun's ultimate solution was to build PumpSwap and control both the launchpad and the DEX. This works but is centralized — it requires vertical integration and prevents composability with other DEXs.

## 5. Proposed Solution: Signer-Verified Config Seeds

### 5.1 Design

Add a **config** account to the pool PDA derivation. The config must be a **signer** on the create_pool transaction:

```
pool_address = PDA(["pool_seed", config.key(), token_mint.key()])
```

The config is not an on-chain state account — it's a pubkey used as a namespace seed. Because it must sign, no one can use someone else's config.

### 5.2 Usage

**Protocol migration (CPI):**
- Config = protocol's PDA (e.g., `PDA(["torch_config"], TORCH_PROGRAM_ID)`)
- Protocol signs via `CpiContext::new_with_signer`
- No one else can produce this signature

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
|----------|--------|
| Frontrun resistant | Yes — can't sign for someone else's config |
| Permissionless | Yes — anyone can create pools in their namespace |
| Deterministic | Yes — PDA from (config, mint) is unique and known |
| No fragmentation | Each namespace has exactly one pool per token |
| No centralization | No admin, no governance, no whitelist |
| Zero additional cost | One extra seed in PDA derivation |
| Backward compatible | Existing pools unaffected by upgrade |

### 5.4 Why Signing Matters

Without signing, anyone can pass any pubkey as config bytes — including the target protocol's program ID. The signer requirement provides cryptographic proof of namespace ownership:

- **Wallet configs:** the wallet signs the transaction (standard Solana signing)
- **Program configs:** only the program can produce a valid PDA signature via CPI
- **No impersonation possible:** Solana's signature verification is at the runtime level

### 5.5 Downstream Instructions

Only `create_pool` requires the config signer. All other instructions (swap, add_liquidity, remove_liquidity) read the config from pool state and use it for PDA validation:

```
seeds = ["pool_seed", pool.config, pool.token_mint]
```

No additional signing required for trading or LP operations.

## 6. Implementation

### 6.1 DeepPool (Reference Implementation)

The signer-verified config seed is implemented in DeepPool v2.0.0:

- `create_pool`: config is a `Signer` account, stored in Pool state
- Pool PDA: `["deep_pool", config, token_mint]`
- All downstream instructions validate PDA using stored `pool.config`
- 16 Kani formal verification proofs cover all math
- Deployed and tested on Solana devnet

### 6.2 Torch Market Integration

Torch Market defines a config PDA (`["torch_config"]`) under its program ID. During migration, Torch CPIs into DeepPool with the config PDA as signer. The pool PDA is deterministic and unfrontrunnable.

### 6.3 Standalone Usage

For non-protocol users creating pools directly, the config is their wallet pubkey. They're already signing the transaction. No additional UX friction.

## 7. Responsible Disclosure

This paper documents a griefing vulnerability class affecting multiple production protocols. The specific attack vector (PDA squatting via bonding curve token purchase + premature pool creation on CPMM) has not been publicly documented. Prior work by Fuzzing Labs (January 2024) identified `open_time` manipulation on Raydium CLMM — a different program, different attack surface, and different impact (parameter manipulation vs. permanent migration failure).

Affected protocols should consider:
1. Implementing signer-verified config seeds (recommended)
2. Adding CPI origin checks to pool creation
3. Implementing fallback migration paths
4. Monitoring for anomalous pool creation patterns

## 8. Conclusion

Pool initialization griefing is a systemic vulnerability in permissionless DeFi. The attack requires no direct profit motive but causes severe, permanent harm to users (funds trapped) and protocols (migration broken). Every launchpad that migrates to a deterministic-address DEX is affected. Existing mitigations are either centralized (admin-only creation), incomplete (parameter bounds checking), or complex (index-based multiple pools).

Signer-verified config seeds eliminate the vulnerability at the protocol level while preserving permissionless design. The fix is minimal (one additional PDA seed + signer check), composable (any protocol can use it), and cryptographically enforced (no spoofing possible).

The reference implementation is open source: [github.com/mrsirg97-rgb/deep_pool](https://github.com/mrsirg97-rgb/deep_pool)

## References
 
- Fuzzing Labs. "DOS - DeFi Liquidity Pools: The Initialization Vulnerability." January 2024. https://fuzzinglabs.com/raydium-dos-initialization/
- Raydium. CLMM Bug Bounty Details. https://docs.raydium.io/raydium/protocol/bug-bounty-program/clmm-bug-bounty-details
- Pump.fun. "Coin Migration Issue Post-Mortem." May 2024. https://x.com/pumpdotfun/status/1791235050643636303
- PumpSwap Program Documentation. https://deepwiki.com/pump-fun/pump-public-docs/4-pumpswap-program
- Immunefi. "Raydium Tick Manipulation Bugfix Review." 2024. https://immunefi.com/blog/bug-fix-reviews/raydium-tick-manipulation-bugfix-review/

## Authors

Built by the Torch Market team.

**DeepPool reference implementation:**
- Source: [github.com/mrsirg97-rgb/deep_pool](https://github.com/mrsirg97-rgb/deep_pool)
- Program ID: `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`
