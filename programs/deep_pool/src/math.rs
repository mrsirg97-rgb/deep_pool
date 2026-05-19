//! Pure integer arithmetic for DeepPool. No Anchor types, no I/O. Every
//! function returns `Option<T>` — `None` means overflow, which instruction
//! handlers surface as `DeepPoolError::MathOverflow` via `.ok_or(...)?`.
//!
//! This module is the single source of truth for the AMM math. Kani proofs
//! in `kani_proofs.rs` import directly from here, so every property proven
//! is proven against the exact code that runs on-chain — not a replica.
//!
//! Proptests live in `tests/math_proptests.rs` (integration test) so the
//! `proptest!` macro DSL isn't parsed by anchor's `#[program]` safety check.

use crate::constants::*;

/// Swap fee on any input amount: `amount * SWAP_FEE_BPS / FEE_DENOMINATOR` (floor).
pub fn calc_swap_fee(amount: u64) -> Option<u64> {
    amount
        .checked_mul(SWAP_FEE_BPS)?
        .checked_div(FEE_DENOMINATOR)
}

/// Constant-product swap output: `effective_in * output_reserve / (input_reserve + effective_in)`.
/// Used for both SOL→Token (sol in, token out) and Token→SOL (token in, sol out).
pub fn calc_swap_output(effective_in: u64, input_reserve: u64, output_reserve: u64) -> Option<u64> {
    let numerator = (effective_in as u128).checked_mul(output_reserve as u128)?;
    let denominator = (input_reserve as u128).checked_add(effective_in as u128)?;
    Some((numerator.checked_div(denominator)?) as u64)
}

/// LP tokens to mint for a new deposit: `lp_supply * deposit / reserve` (floor).
/// Pro-rata against either side of the pool — caller picks which reserve.
/// Returns `None` if the u128 product overflows OR if the u64 cast would lose
/// bits (i.e. the result exceeds `u64::MAX`).
pub fn calc_lp_mint(lp_supply: u64, deposit: u64, reserve: u64) -> Option<u64> {
    let result = (lp_supply as u128)
        .checked_mul(deposit as u128)?
        .checked_div(reserve as u128)?;
    u64::try_from(result).ok()
}

/// Asset to return on LP burn: `lp_amount * reserve / lp_supply` (floor).
/// Caller picks which reserve (SOL or token) to redeem against.
/// Returns `None` on u128 multiplication overflow or u64 truncation.
pub fn calc_lp_redeem(lp_amount: u64, reserve: u64, lp_supply: u64) -> Option<u64> {
    let result = (lp_amount as u128)
        .checked_mul(reserve as u128)?
        .checked_div(lp_supply as u128)?;
    u64::try_from(result).ok()
}

/// Proportional amount of `reserve_b` matching an input of `reserve_a`:
/// `input * reserve_b / reserve_a` (floor). Used to compute the SOL side
/// of a proportional liquidity deposit given a target token deposit (or
/// vice versa). Same algebra as `calc_lp_redeem` but named for the actual
/// caller intent — avoids semantically misusing the LP helper.
/// Returns `None` on u128 multiplication overflow or u64 truncation.
pub fn calc_proportional(input: u64, reserve_a: u64, reserve_b: u64) -> Option<u64> {
    let result = (input as u128)
        .checked_mul(reserve_b as u128)?
        .checked_div(reserve_a as u128)?;
    u64::try_from(result).ok()
}

/// Integer square root via Newton's method. Used to seed the initial LP supply
/// at pool creation: `sqrt(initial_sol * initial_tokens)`.
pub fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }

    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
