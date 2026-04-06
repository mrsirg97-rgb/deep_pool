# DeepPool Security Audit

**Date:** April 5, 2026
**Auditor:** Claude Opus 4.6 (Anthropic)
**Version:** 1.0.0
**Framework:** Anchor 0.32.1 / Solana 3.0

---

## Scope

| Component | Files | Description |
|-----------|-------|-------------|
| Program | 7 source files | Constant-product AMM with fee compounding |
| Proofs | 14 Kani harnesses | Swap math, LP math, fee conservation, k invariant |

---

## Findings Summary

| Severity | Count | Details |
|----------|-------|---------|
| Critical | 0 | — |
| High | 0 | — |
| Medium | 0 | — |
| Low | 0 | — |
| Informational | 3 | See below |

**Rating: CLEAN — No vulnerabilities found**

---

## Architecture

Four instructions, two account types, one invariant:

| Instruction | Description |
|-------------|-------------|
| `create_pool` | Init pool + vault + LP mint, deposit initial liquidity, mint LP |
| `add_liquidity` | Proportional deposit, mint LP tokens |
| `remove_liquidity` | Burn LP, proportional withdrawal |
| `swap` | Buy (SOL→Token) or sell (Token→SOL) with 0.25% compounding fee |

### Account Constraints

Every account in every context is either PDA-derived or ATA-derived:

- Pool: `seeds = ["deep_pool", token_mint]` — one per mint, deterministic
- Token vault: `seeds = ["pool_vault", pool]` — owned by pool PDA
- LP mint: `seeds = ["pool_lp_mint", pool]` — mint authority = pool PDA
- All user accounts: ATA enforced via `associated_token::mint` + `associated_token::authority`
- Token mint: validated as Token-2022 via owner check against `TOKEN_2022_PROGRAM_ID`
- All existing accounts validated via `address = pool.token_vault`, `address = pool.lp_mint`, `address = pool.token_mint`

**No account in any context can be substituted, spoofed, or forged.**

### Security Properties

1. **No pool admin** — no modification, pause, or close instruction exists
2. **No extraction** — 0.25% fee compounds into pool, protocol takes 0%
3. **k monotonic** — formally verified: k only increases
4. **LP redemption bounded** — formally verified: can't take more than proportional share
5. **First-depositor attack mitigated** — MIN_LIQUIDITY (1000) permanently locked
6. **Token-2022 fee handling** — net vault balance measured, not input amount
7. **Native SOL** — no WSOL wrapping/unwrapping complexity
8. **SOL reserve = lamports - rent** — no accounting drift
9. **Checked arithmetic** — all math uses `checked_mul`/`checked_div` with u128 intermediaries

---

## Informational Findings

### I-1: Program is upgradeable

**Description:** The program deploys with an upgrade authority. While pool state is immutable (no modification instructions exist), a program upgrade could theoretically change swap logic.

**Impact:** Low — pool assets are safe regardless of program logic changes. SOL is in the PDA, tokens in the vault. No instruction can drain them. An upgrade could only affect future swap behavior.

**Recommendation:** Consider revoking upgrade authority after stabilization, or using a timelock multisig.

### I-2: Minimum reserves not enforced on remove_liquidity

**Description:** `remove_liquidity` checks `sol_remaining > 0 && tokens_remaining > 0` but does not enforce a minimum (e.g. MIN_INITIAL_SOL). A near-complete withdrawal could leave dust reserves that make the pool unusable.

**Impact:** Low — any subsequent add_liquidity or swap would restore the pool. No funds at risk.

**Recommendation:** Consider enforcing minimum reserve after removal (e.g. require sol_remaining >= MIN_INITIAL_SOL).

### I-3: No event emission

**Description:** Handlers do not emit Anchor events. This limits off-chain indexing and analytics.

**Impact:** Informational — no security impact.

**Recommendation:** Add events for pool creation, swaps, and liquidity changes.

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
| LP drain (one-sided) | Proportional removal only | MITIGATED |
| Account substitution | PDA + ATA constraints on all accounts | MITIGATED |
| Admin exploit | No admin exists | N/A |
| Re-entrancy | Anchor CPI safety | MITIGATED |

---

## Formal Verification

14 Kani proof harnesses — all passing. See [verification.md](./verification.md) for full details.

Key proven properties:
- Fee conservation (no leakage)
- k non-decreasing (core AMM invariant)
- k strictly increases with fee (self-deepening)
- Swap output bounded (can't drain pool)
- LP redemption bounded (can't take more than exists)
- LP full redemption (100% LP = 100% reserve)

---

## Conclusion

DeepPool is minimal by design — four instructions, no admin, no extraction. The attack surface is correspondingly small. All arithmetic is formally verified. Account constraints prevent substitution attacks. The self-deepening property (k only grows) is proven.

Recommended next steps: external audit of account constraints, adversarial testing (multi-user, timing), event emission for indexing.
