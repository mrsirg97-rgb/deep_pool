# DeepPool Property Testing Report

## Overview

DeepPool's core arithmetic is property-tested using [proptest](https://proptest-rs.github.io/proptest/), a Rust fuzz-style property testing framework. Properties cover swap math, fee conservation, LP minting and redemption, integer square root, and multi-step compositional invariants (swap-roundtrip and LP-roundtrip no-extraction).

Proptest complements the Kani harnesses ([verification.md](./verification.md)): Kani proves exact correctness at concrete values, proptest explores the full u64 input space with thousands of randomly-drawn cases and automatically shrinks any failing input down to the minimal reproducing case.

**Tool:** proptest 1.x
**Target:** `deep_pool` v3.1.0
**Properties:** 19 properties across 6 modules, all passing
**Cases per property:** 10,000 (5,000 for the composite roundtrip)
**Total assertions per run:** ~185,000
**Source:** `programs/deep_pool/tests/math_proptests.rs`
**Run with:** `cargo test -p deep_pool --test math_proptests`

Located under `tests/` rather than `src/` so the `proptest!` macro DSL isn't parsed by Anchor's `#[program]` safety-check macro, which walks the lib source tree with syn and doesn't understand macro semantics.

## What Is Verified

### Swap Fee (Properties 1-3)

| Property | Cases | Description |
|----------|-------|-------------|
| `swap_fee_never_panics_and_is_bounded` | 10,000 | For all u64 inputs, `calc_swap_fee` either returns `None` or a value `≤ input` and exactly equal to `input * 25 / 10000` (u128 intermediate) |
| `swap_fee_monotonic` | 10,000 | `fee(a) ≤ fee(b)` whenever `a ≤ b`, across all pairs |
| `swap_fee_conservation` | 10,000 | `fee(x) + (x - fee(x)) == x` for all valid inputs |

### Constant Product Swap (Properties 4-7)

| Property | Cases | Description |
|----------|-------|-------------|
| `swap_output_bounded_by_reserve` | 10,000 | `calc_swap_output(...) < output_reserve` for all reserves up to 10^18 and all positive inputs |
| `swap_output_zero_input_is_zero` | 10,000 | Zero input produces zero output across all reserve combinations |
| `swap_output_monotonic_in_input` | 10,000 | Larger input produces ≥ output (monotonicity holds across the entire reserve range) |
| `swap_k_non_decreasing` | 10,000 | For any swap with fee, `K_after ≥ K_before` where `K = input_reserve × output_reserve` — the self-deepening invariant |

### LP Mint (Properties 8-11)

| Property | Cases | Description |
|----------|-------|-------------|
| `lp_mint_zero_deposit_is_zero` | 10,000 | Zero deposit mints zero LP across all supply/reserve combos |
| `lp_mint_full_deposit_equals_supply` | 10,000 | Depositing exactly the reserve amount mints exactly the current LP supply |
| `lp_mint_monotonic_in_deposit` | 10,000 | Larger deposit mints ≥ LP |
| `lp_mint_bounded_when_deposit_le_reserve` | 10,000 | When deposit ≤ reserve, minted LP ≤ existing supply (no inflation) |

### LP Redeem (Properties 12-15)

| Property | Cases | Description |
|----------|-------|-------------|
| `lp_redeem_zero_is_zero` | 10,000 | Burning zero LP withdraws zero reserve |
| `lp_redeem_full_supply_equals_reserve` | 10,000 | Burning the entire LP supply withdraws the entire reserve |
| `lp_redeem_bounded_by_reserve` | 10,000 | Any LP fraction withdraws ≤ reserve |
| `lp_mint_redeem_roundtrip_no_extraction` | 10,000 | Deposit → mint LP → burn that same LP → output ≤ deposit. Roundtripping through the LP math extracts no value. |

### Integer Square Root (Properties 16-18)

| Property | Cases | Description |
|----------|-------|-------------|
| `integer_sqrt_is_floor` | 10,000 | `integer_sqrt(n)² ≤ n < (integer_sqrt(n) + 1)²` — exactly the floored square root |
| `integer_sqrt_monotonic` | 10,000 | `sqrt(a) ≤ sqrt(b)` when `a ≤ b` |
| `integer_sqrt_perfect_squares` | 10,000 | `integer_sqrt(s²) == s` for all `s` in `[0, u32::MAX]` |

### Composite (Property 19)

| Property | Cases | Description |
|----------|-------|-------------|
| `swap_roundtrip_no_extraction` | 5,000 | Buy tokens then sell them back (both with fee). Output SOL ≤ input SOL across all input sizes and reserve configurations. Round-tripping a swap cannot extract value from the pool. |

## Why Proptest Alongside Kani

The two tools cover different ground:

**Kani (model checking)** — Exhaustive proofs at *concrete* representative values. Proves exact correctness for the values tested. Struggles with free u64 symbolic inputs flowing through u128 multiply/divide chains because SAT solvers can't handle that efficiently.

**Proptest (property-based fuzzing)** — Randomly samples the input space with smart shrinking. Covers 10,000 distinct cases per property, which finds violations that exhaustive-at-concrete-values testing might miss. Doesn't prove exhaustive correctness but greatly widens the empirical coverage.

Together: Kani pins down the behavior at the edges and representative points; proptest sweeps the middle and catches anything that slips between Kani's concrete probes.

**Regression durability:** proptest automatically writes failing seeds to `proptest-regressions/` on failure, and replays them on every subsequent run. Any future regression on a property that fired in the past will be caught deterministically.

## What Is NOT Verified

The same exclusions as Kani — neither tool covers:

- Access control (account constraints, PDA ownership, Signer checks)
- CPI safety (Token-2022 transfer interactions, transfer hook reentrancy)
- Economic attacks (sandwich, front-running, MEV)
- Rent-exempt minimum handling edge cases
- Network-level concerns (transaction ordering, commitment levels)
- The new v3.1.0 `sol_source` account substitution surface — covered by code audit and Anchor's `Signer` constraint

These require code audit and adversarial testing. See [audit.md](./audit.md).

## Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `SWAP_FEE_BPS` | 25 | 0.25% swap fee |
| `FEE_DENOMINATOR` | 10000 | BPS denominator |
| `MIN_LIQUIDITY` | 1000 | Subtracted from initial sqrt (anti first-depositor floor) |
| `MIN_INITIAL_SOL` | 100,000,000 | 0.1 SOL minimum |
| `MIN_INITIAL_TOKENS` | 1,000,000 | 1 token minimum (6 decimals) |
| `RESERVE_MAX` (proptest) | 10^18 | Upper bound on reserve ranges — exceeds any realistic pool size |
