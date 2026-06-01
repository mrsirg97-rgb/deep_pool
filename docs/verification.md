# DeepPool Formal Verification Report

## Overview

DeepPool's core arithmetic is formally verified using [Kani](https://model-checking.github.io/kani/), a Rust model checker backed by the CBMC bounded model checker. Proofs cover swap math, fee conservation, LP minting/redemption, and the self-deepening invariant (k monotonically non-decreasing).

**Tool:** Kani Rust Verifier 0.67.0 / CBMC 6.8.0
**Target:** `deep_pool` v7.0.0
**Harnesses:** 25 proof harnesses (24 concrete + 1 symbolic), all passing
**Source:** `programs/deep_pool/src/kani_proofs.rs`
**Companion:** [properties.md](./properties.md) — 31 proptest properties for broader random coverage

> **v4.0.0 note.** v4 added `emit_cpi!` event emission to all four instructions. Events are observability, not protocol logic — they don't touch the math verified here. The original 16 Kani harnesses pass unchanged against the v4.0.0 binary.
>
> **v4.2.0 note.** Added 5 new harnesses: 3 for the new `calc_proportional` helper (zero-input, identity, overflow-returns-`None`) and 2 sibling-overflow proofs (`calc_lp_mint`, `calc_lp_redeem`) covering the `u64::try_from` hardening that closes silent u128→u64 truncation. Constant-product math itself is unchanged; the additions verify the math-API hygiene fixes from the v4.2 audit.
>
> **v5.0.0 note.** Jupiter-readiness hardening pass — Token-2022 extension blocklist, explicit `token_program` constraint, explicit rent-exempt assertion at the swap-sell lamport site, and account-list cleanup. Pure-math surface is unchanged; all 21 harnesses pass against the v5.0.0 binary without modification.
>
> **v6.0.0 note.** Added the in-pool TWAP oracle. 3 new harnesses (22-24) verify the oracle's pure math: `price_q64` exactness + `None`-on-zero-denominator, and `accumulate_price` window-difference exactness — including a deliberate past-2^128 wrap, proving the wrapping cumulative differences correctly (the property the mark depends on). The constant-product/LP math is untouched; all 21 prior harnesses pass unchanged. The oracle's ring-selection / lazy-extension logic (`read_twap_sol_per_tok`) is *not* a Kani target (it traverses a fixed array — see "What Is NOT Verified"); it's covered by proptest + litesvm instead.
>
> **v7.0.0 note.** Four hardening changes (`create_pool` `sol_source` separation; swap-fee min-1 floor; `add_liquidity` SOL charge rounds UP; TWAP read fails closed on a sub-floor pool or a degenerate `0` mark). 1 new harness — `verify_proportional_ceil_rounds_up` — proves `calc_proportional_ceil ∈ {floor, floor+1}` (the pool-favoring rounding for the deposit's SOL charge), bringing the total to 25. `verify_swap_fee_threshold` was updated to the min-1 semantics (no nonzero swap is fee-free). The sub-floor/`0`→`None` read guards are proptest+litesvm-covered (the read traverses a fixed array; not a Kani target). All prior harnesses pass unchanged.

## What Is Verified

### Swap Fee (Harnesses 1-3)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_swap_fee_conservation` | Concrete | fee + effective = input at all scales (1 lamport to 1000 SOL) |
| `verify_swap_fee_threshold` | Concrete | min-1 floor: fee = 0 only at amount 0, fee = 1 for any nonzero swap up to 400 lamports, fee = 2,500,000 at 1 SOL |
| `verify_swap_fee_bounded_symbolic` | **Symbolic** | fee ≤ amount for ALL u64 inputs |

### Constant Product Swap (Harnesses 4-8)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_swap_output_bounded` | Concrete | output < reserve for all trade sizes (200 SOL pool) |
| `verify_swap_output_bounded_large_pool` | Concrete | output < reserve for all trade sizes (1000 SOL pool) |
| `verify_k_non_decreasing` | Concrete | k_after >= k_before for all swaps with fee |
| `verify_swap_monotonic` | Concrete | larger input produces larger output (5 orders of magnitude + adjacent) |
| `verify_swap_zero_input` | Concrete | zero input = zero output |

### Sell-Side (Harness 9)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_sell_output_bounded` | Concrete | sell-side output < SOL reserve for all token inputs |

### LP Token Math (Harnesses 10-14)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_initial_lp_sqrt` | Concrete | sqrt correct at min, typical, and max pool sizes; sqrt > MIN_LIQUIDITY |
| `verify_lp_mint_proportional` | Concrete | 1% deposit = 1% LP, 100% deposit = 100% LP, dust = 0 LP |
| `verify_lp_redeem_bounded` | Concrete | redeemed <= reserve at all redemption sizes |
| `verify_lp_full_redeem` | Concrete | 100% LP = 100% reserve at multiple reserve sizes |
| `verify_lp_redeem_monotonic` | Concrete | more LP = more output (1% < 10% < 50% < 100%) |

### Fee Compounding (Harness 15)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_fee_compounds_k` | Concrete | k strictly increases when fee > 0 (proven at 4 swap sizes) |

This is the self-deepening property: every fee-generating swap makes the pool deeper.

### LP Lock Rates (Harness 16)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_lp_lock_rates` | Concrete | Creator: exactly 20%/80% split. Provider: exactly 7.5%/92.5% split. Conservation holds at all scales. |

### TWAP Oracle (Harnesses 22-24, v6.0.0)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_price_q64_at_scale` | Concrete | `price_q64(out, in)` never panics (the `u64 << 64` shift fits u128), is `None` iff `in == 0`, else exactly `(out << 64) / in` — across low/high/extreme reserve ratios |
| `verify_accumulate_price_window_difference` | Concrete | accumulating over a slot sequence and taking `wrapping_sub` from the start recovers the exact summed contribution (the read-mark invariant) |
| `verify_accumulate_price_wraps_exactly` | Concrete | when the running head wraps past `u128::MAX`, the window `wrapping_sub` is still exact — this is why the cumulative may grow unbounded mod 2^128 |

`price_q64` divides, so a symbolic harness would explode the SAT solver — concrete values are used, matching the rest of the suite. These cover the *pure* oracle math; the ring traversal and lazy head-extension are exercised by proptest + litesvm (`twap::*`).

## Symbolic vs Concrete

Proofs marked **Symbolic** use `kani::any()` — they verify the property for every possible input within the type's range. The swap fee proof works symbolically because it's pure u64 arithmetic.

Proofs marked **Concrete** use specific representative values spanning the protocol's operating range (dust to 1000 SOL, 1 token to 500M tokens). These cover the constant-product math which uses u128 intermediate arithmetic — CBMC's SAT solver cannot handle multiple symbolic u64 inputs flowing through u128 multiply+divide chains within reasonable time.

The concrete approach matches how the protocol actually operates: pools range from 0.1 SOL (minimum) to thousands of SOL, and the proofs verify correctness at every scale within that range.

## What Is NOT Verified

- Access control (account constraints, PDA ownership)
- CPI safety (Token-2022 transfer interactions)
- Economic attacks (sandwich, front-running)
- Rent-exempt minimum handling
- Network-level concerns (transaction ordering)
- TWAP ring traversal / lazy head-extension (`read_twap_sol_per_tok`) — array-walking + clock arithmetic, not pure number theory; covered by proptest + litesvm (`twap::*`). Only the oracle's pure primitives (`price_q64`, `accumulate_price`) are Kani targets.

These require code audit and adversarial testing, not formal verification. The oracle's economic/manipulation surface and its **consumer contract** are analyzed in [audit.md](./audit.md) I-8–I-11.

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `SWAP_FEE_BPS` | 25 | 0.25% swap fee |
| `FEE_DENOMINATOR` | 10000 | BPS denominator |
| `MIN_LIQUIDITY` | 1000 | Locked on first deposit |
| `MIN_INITIAL_SOL` | 100,000,000 | 0.1 SOL minimum |
| `MIN_INITIAL_TOKENS` | 1,000,000 | 1 token minimum (6 decimals) |
