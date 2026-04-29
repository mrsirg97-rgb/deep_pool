# DeepPool Formal Verification Report

## Overview

DeepPool's core arithmetic is formally verified using [Kani](https://model-checking.github.io/kani/), a Rust model checker backed by the CBMC bounded model checker. Proofs cover swap math, fee conservation, LP minting/redemption, and the self-deepening invariant (k monotonically non-decreasing).

**Tool:** Kani Rust Verifier 0.67.0 / CBMC 6.8.0
**Target:** `deep_pool` v4.0.0
**Harnesses:** 16 proof harnesses (15 concrete + 1 symbolic), all passing
**Source:** `programs/deep_pool/src/kani_proofs.rs`
**Companion:** [properties.md](./properties.md) — 19 proptest properties for broader random coverage

> **v4.0.0 note.** v4 added `emit_cpi!` event emission to all four instructions. Events are observability, not protocol logic — they don't touch the math verified here. The 16 Kani harnesses are unchanged from v3.1.0 and continue to pass against the v4.0.0 binary.

## What Is Verified

### Swap Fee (Harnesses 1-3)

| Harness | Method | Property |
|---------|--------|----------|
| `verify_swap_fee_conservation` | Concrete | fee + effective = input at all scales (1 lamport to 1000 SOL) |
| `verify_swap_fee_threshold` | Concrete | fee = 0 below 400 lamports, fee = 1 at 400, fee = 2,500,000 at 1 SOL |
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

These require code audit and adversarial testing, not formal verification.

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `SWAP_FEE_BPS` | 25 | 0.25% swap fee |
| `FEE_DENOMINATOR` | 10000 | BPS denominator |
| `MIN_LIQUIDITY` | 1000 | Locked on first deposit |
| `MIN_INITIAL_SOL` | 100,000,000 | 0.1 SOL minimum |
| `MIN_INITIAL_TOKENS` | 1,000,000 | 1 token minimum (6 decimals) |
