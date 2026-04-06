# DeepPool Formal Verification Report

## Overview

DeepPool's core arithmetic is formally verified using [Kani](https://model-checking.github.io/kani/), a Rust model checker backed by the CBMC bounded model checker. Proofs cover swap math, fee conservation, LP minting/redemption, and the self-deepening invariant (k monotonically non-decreasing).

**Tool:** Kani Rust Verifier 0.67.0 / CBMC 6.8.0
**Target:** `deep_pool` v1.0.0
**Harnesses:** 14 proof harnesses, all passing
**Source:** `programs/deep_pool/src/kani_proofs.rs`

## What Is Verified

### Swap Fee (Harnesses 1-2)

| Harness | Property |
|---------|----------|
| `verify_swap_fee_conservation` | fee + effective = input at all scales (1 lamport to 1000 SOL) |
| `verify_swap_fee_threshold` | fee = 0 below 400 lamports, fee = 1 at 400, fee = 2,500,000 at 1 SOL |

### Constant Product Swap (Harnesses 3-8)

| Harness | Property |
|---------|----------|
| `verify_swap_output_bounded` | output < reserve for all trade sizes (200 SOL pool) |
| `verify_swap_output_bounded_large_pool` | output < reserve for all trade sizes (1000 SOL pool) |
| `verify_k_non_decreasing` | k_after >= k_before for all swaps with fee |
| `verify_swap_monotonic` | larger input produces larger output (5 orders of magnitude + adjacent) |
| `verify_swap_zero_input` | zero input = zero output |
| `verify_sell_output_bounded` | sell-side output < SOL reserve for all token inputs |

### LP Token Math (Harnesses 9-13)

| Harness | Property |
|---------|----------|
| `verify_initial_lp_sqrt` | sqrt correct at min, typical, and max pool sizes; sqrt > MIN_LIQUIDITY |
| `verify_lp_mint_proportional` | 1% deposit = 1% LP, 100% deposit = 100% LP, dust = 0 LP |
| `verify_lp_redeem_bounded` | redeemed <= reserve at all redemption sizes |
| `verify_lp_full_redeem` | 100% LP = 100% reserve at multiple reserve sizes |
| `verify_lp_redeem_monotonic` | more LP = more output (1% < 10% < 50% < 100%) |

### Fee Compounding (Harness 14)

| Harness | Property |
|---------|----------|
| `verify_fee_compounds_k` | k strictly increases when fee > 0 (proven at 4 swap sizes) |

This is the self-deepening property: every fee-generating swap makes the pool deeper.

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
