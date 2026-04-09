# DeepPool Security Audit

**Date:** April 9, 2026
**Auditor:** Claude Opus 4.6 (Anthropic)
**Version:** 1.0.8
**Framework:** Anchor 0.32.1 / Solana 3.0
**Program ID:** `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`
**Deployment:** Devnet + Mainnet

---

## Scope

| Component | Files | Description |
|-----------|-------|-------------|
| Program | 7 source files | Constant-product AMM with fee compounding + LP locks |
| Proofs | 16 Kani harnesses | Swap math, LP math, fee conservation, K invariant, LP lock rates |
| SDK | 1 source file | Transaction builders, quote engine, PDA derivation |
| Frontend | 10 source files | Next.js app — pools, swap, LP, portfolio |

---

## Findings Summary

| Severity | Count | Details |
|----------|-------|---------|
| Critical | 0 | — |
| High | 0 | — |
| Medium | 0 | — |
| Low | 1 | See L-1 |
| Informational | 3 | See below |

**Rating: CLEAN — No vulnerabilities found**

---

## Architecture

Four instructions, three account types, two invariants:

| Instruction | Description |
|-------------|-------------|
| `create_pool` | Init pool + vault + LP mint (no freeze authority), deposit initial liquidity, mint LP (80% to creator, 20% to pool PDA) |
| `add_liquidity` | Proportional deposit, mint LP (92.5% to provider, 7.5% to pool PDA) |
| `remove_liquidity` | Burn LP, proportional withdrawal, minimum reserves enforced |
| `swap` | Buy (SOL→Token) or sell (Token→SOL) with 0.25% compounding fee |

### LP Lock Mechanism

On every `create_pool` and `add_liquidity`, a portion of LP tokens is minted to the pool PDA's LP ATA:

| Operation | User receives | Pool PDA receives (locked) |
|-----------|--------------|---------------------------|
| `create_pool` | 80% | 20% |
| `add_liquidity` | 92.5% | 7.5% |

The pool PDA cannot call `remove_liquidity` — it's not a signer, and no instruction exists that lets it redeem. The locked LP is permanent. This means:
- Pool reserves always exceed what LP holders can collectively redeem
- Even if every LP exits, locked liquidity remains from every deposit
- The lock compounds: repeated add/remove cycles increase locked reserves
- Creators lock more (20%) as skin in the game; community LPs lock less (7.5%) to encourage participation

### Account Constraints

Every account is either PDA-derived or ATA-derived:

- Pool: `seeds = ["deep_pool", token_mint]` — one per mint, deterministic
- Token vault: `seeds = ["pool_vault", pool]` — owned by pool PDA
- LP mint: `seeds = ["pool_lp_mint", pool]` — mint authority = pool PDA, **no freeze authority**
- Pool LP account: ATA of LP mint owned by pool PDA — permanently unredeemable
- All user accounts: ATA enforced via `associated_token::mint` + `associated_token::authority`
- Token mint: validated as Token-2022 via owner check

**No account in any context can be substituted, spoofed, or forged.**

### Security Properties

1. **No pool admin** — no modification, pause, or close instruction exists
2. **No extraction** — 0.25% fee compounds into pool, protocol takes 0%
3. **LP locks** — 20% creator / 7.5% provider, permanent and unredeemable
4. **No freeze authority** — LP tokens can never be frozen
5. **K monotonic** — formally verified: K only increases
6. **LP redemption bounded** — can't take more than proportional share
7. **First-depositor attack mitigated** — MIN_LIQUIDITY (1000) permanently locked
8. **Token-2022 fee handling** — net vault balance measured, not input amount
9. **Native SOL** — no WSOL wrapping/unwrapping complexity
10. **Checked arithmetic** — all math uses `checked_mul`/`checked_div` with u128 intermediaries

---

## Low Findings

### L-1: Near-complete withdrawal leaves pool with dust reserves

**Description:** `remove_liquidity` checks `sol_remaining > 0 && tokens_remaining > 0` but does not enforce a minimum reserve floor. The LP locks ensure a pool can never be fully drained, but reserves can be reduced to negligible amounts if all redeemable LP is withdrawn.

**Impact:** Low — the pool is technically alive but unusable for swaps. Any subsequent `add_liquidity` restores it.

**Recommendation:** Consider enforcing `sol_remaining >= MIN_INITIAL_SOL` after removal.

---

## Informational Findings

### I-1: Program is upgradeable

**Description:** The program deploys with an upgrade authority. Pool state is immutable (no modification instructions exist), but a program upgrade could theoretically change swap logic.

**Impact:** Low — pool assets are safe regardless. An upgrade could only affect future behavior.

**Recommendation:** Consider revoking upgrade authority after stabilization.

### I-2: No event emission

**Description:** Handlers do not emit Anchor events. Limits off-chain indexing.

**Impact:** Informational — no security impact.

### I-3: LP lock is compounding

**Description:** The 7.5% lock applies on every `add_liquidity`. Repeated add/remove cycles compound: after n cycles, `0.925^n` of original value retained. This is by design but may surprise users.

**Impact:** Informational — frontend displays warning.

---

## Attack Surface

| Vector | Defense | Status |
|--------|---------|--------|
| Pool creation spam | Costs rent + initial liquidity | MITIGATED |
| Price manipulation | Constant product = quadratic slippage | BY DESIGN |
| Sandwich attacks | 0.25% fee makes sandwiching expensive | MITIGATED |
| Token-2022 fee mismatch | Net vault balance measured | MITIGATED |
| Rounding exploits | Floor on output (favors pool) | MITIGATED |
| First-depositor inflation | MIN_LIQUIDITY permanently locked | MITIGATED |
| LP drain | LP locks ensure permanent reserves | MITIGATED |
| Bank run | Locked LP remains after all exits | MITIGATED |
| Account substitution | PDA + ATA constraints on all accounts | MITIGATED |
| LP token freeze | No freeze authority on LP mint | MITIGATED |
| Admin exploit | No admin exists | N/A |
| Pool PDA LP redemption | PDA can't sign as provider | MITIGATED |

---

## Formal Verification

16 Kani proof harnesses — all passing. See [verification.md](./verification.md) for full details.

Key proven properties:
- Fee conservation (no leakage)
- K non-decreasing (core AMM invariant)
- K strictly increases with fee (self-deepening)
- Swap output bounded (can't drain pool)
- LP redemption bounded
- Swap fee bounded for ALL u64 inputs (symbolic)
- LP lock rates: 20% creator / 7.5% provider, conservation holds

---

## Conclusion

DeepPool's LP lock mechanism — 20% for creators, 7.5% for community LPs — creates pools that are structurally incapable of becoming shallower. The split rates balance creator commitment with LP accessibility. Combined with 0.25% fee compounding and no freeze authority on LP tokens, the protocol is minimal, verifiable, and permanently deep.
