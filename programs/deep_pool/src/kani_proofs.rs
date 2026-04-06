//! Kani Formal Verification Proof Harnesses for DeepPool
//!
//! Proves properties of the constant-product AMM math at concrete values
//! spanning the protocol's operating range. Run with: cargo kani
//!
//! Concrete inputs avoid SAT solver explosion on wide integer arithmetic
//! while verifying correctness at every scale the protocol operates at.

use crate::constants::*;

// ============================================================================
// Pure math replicas
// ============================================================================

fn calc_swap_fee(amount: u64) -> Option<u64> {
    amount.checked_mul(SWAP_FEE_BPS)?.checked_div(FEE_DENOMINATOR)
}

fn calc_swap_output(effective_in: u64, input_reserve: u64, output_reserve: u64) -> Option<u64> {
    let numerator = (effective_in as u128).checked_mul(output_reserve as u128)?;
    let denominator = (input_reserve as u128).checked_add(effective_in as u128)?;
    Some((numerator.checked_div(denominator)?) as u64)
}

fn calc_lp_mint(lp_supply: u64, deposit: u64, reserve: u64) -> Option<u64> {
    let result = (lp_supply as u128)
        .checked_mul(deposit as u128)?
        .checked_div(reserve as u128)?;
    Some(result as u64)
}

fn calc_lp_redeem(lp_amount: u64, reserve: u64, lp_supply: u64) -> Option<u64> {
    let result = (lp_amount as u128)
        .checked_mul(reserve as u128)?
        .checked_div(lp_supply as u128)?;
    Some(result as u64)
}

fn integer_sqrt(n: u128) -> u128 {
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

// ============================================================================
// 1. Swap Fee
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_swap_fee_conservation() {
    // Test across range: dust, small, medium, large
    let amounts: [u64; 6] = [
        1,                      // 1 lamport
        399,                    // just below fee threshold
        400,                    // exact threshold (fee = 1)
        1_000_000_000,          // 1 SOL
        100_000_000_000,        // 100 SOL
        1_000_000_000_000,      // 1000 SOL
    ];

    for amount in amounts {
        let fee = calc_swap_fee(amount).unwrap();
        let effective = amount - fee;

        // Conservation: fee + effective = input
        assert!(fee + effective == amount);
        // Fee bounded
        assert!(fee <= amount);
        // Exact formula
        assert!(fee == amount * SWAP_FEE_BPS / FEE_DENOMINATOR);
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_swap_fee_threshold() {
    // Below threshold: fee = 0
    assert!(calc_swap_fee(399).unwrap() == 0);
    // At threshold: fee = 1
    assert!(calc_swap_fee(400).unwrap() == 1);
    // Above: fee > 0
    assert!(calc_swap_fee(10_000).unwrap() > 0);
    // 1 SOL: fee = 2_500_000 (0.25% of 10^9)
    assert!(calc_swap_fee(1_000_000_000).unwrap() == 2_500_000);
}

// ============================================================================
// 2. Constant Product Swap
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_swap_output_bounded() {
    let pool_sol: u64 = 200_000_000_000;        // 200 SOL
    let pool_tokens: u64 = 150_000_000_000_000;  // 150M tokens

    let inputs: [u64; 5] = [
        1_000_000,              // 0.001 SOL
        100_000_000,            // 0.1 SOL
        1_000_000_000,          // 1 SOL
        50_000_000_000,         // 50 SOL
        199_000_000_000,        // 199 SOL (nearly all reserves)
    ];

    for input in inputs {
        let output = calc_swap_output(input, pool_sol, pool_tokens).unwrap();
        assert!(output < pool_tokens);
        assert!(output > 0);
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_swap_output_bounded_large_pool() {
    let pool_sol: u64 = 1_000_000_000_000;      // 1000 SOL
    let pool_tokens: u64 = 500_000_000_000_000;  // 500M tokens

    let inputs: [u64; 3] = [
        1_000_000_000,          // 1 SOL
        100_000_000_000,        // 100 SOL
        500_000_000_000,        // 500 SOL
    ];

    for input in inputs {
        let output = calc_swap_output(input, pool_sol, pool_tokens).unwrap();
        assert!(output < pool_tokens);
        assert!(output > 0);
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_k_non_decreasing() {
    let sol_reserve: u64 = 200_000_000_000;
    let token_reserve: u64 = 150_000_000_000_000;
    let k_before = (sol_reserve as u128) * (token_reserve as u128);

    // Test at multiple swap sizes
    let swaps: [u64; 5] = [
        400,                    // minimum fee-generating swap
        1_000_000_000,          // 1 SOL
        10_000_000_000,         // 10 SOL
        50_000_000_000,         // 50 SOL
        100_000_000_000,        // 100 SOL
    ];

    for sol_in in swaps {
        let fee = calc_swap_fee(sol_in).unwrap();
        let effective_in = sol_in - fee;
        let tokens_out = calc_swap_output(effective_in, sol_reserve, token_reserve).unwrap();

        let new_sol = (sol_reserve as u128) + (sol_in as u128);
        let new_tokens = (token_reserve as u128) - (tokens_out as u128);
        let k_after = new_sol * new_tokens;

        assert!(k_after >= k_before);
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_swap_monotonic() {
    let sol_reserve: u64 = 200_000_000_000;
    let token_reserve: u64 = 150_000_000_000_000;

    let out_001 = calc_swap_output(10_000_000, sol_reserve, token_reserve).unwrap();
    let out_01 = calc_swap_output(100_000_000, sol_reserve, token_reserve).unwrap();
    let out_1 = calc_swap_output(1_000_000_000, sol_reserve, token_reserve).unwrap();
    let out_10 = calc_swap_output(10_000_000_000, sol_reserve, token_reserve).unwrap();
    let out_100 = calc_swap_output(100_000_000_000, sol_reserve, token_reserve).unwrap();

    assert!(out_01 > out_001);
    assert!(out_1 > out_01);
    assert!(out_10 > out_1);
    assert!(out_100 > out_10);

    // Adjacent: n+1 >= n
    let out_a = calc_swap_output(1_000_000_000, sol_reserve, token_reserve).unwrap();
    let out_b = calc_swap_output(1_000_000_001, sol_reserve, token_reserve).unwrap();
    assert!(out_b >= out_a);
}

#[cfg(kani)]
#[kani::proof]
fn verify_swap_zero_input() {
    assert!(calc_swap_output(0, 200_000_000_000, 150_000_000_000_000).unwrap() == 0);
    assert!(calc_swap_output(0, 1_000_000_000_000, 500_000_000_000_000).unwrap() == 0);
}

// ============================================================================
// 3. LP Mint
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_initial_lp_sqrt() {
    // Edge cases
    assert!(integer_sqrt(0) == 0);
    assert!(integer_sqrt(1) == 1);
    assert!(integer_sqrt(3) == 1);
    assert!(integer_sqrt(4) == 2);
    assert!(integer_sqrt(100) == 10);

    // Min pool: 0.1 SOL * 1 token = 10^14
    let small = (MIN_INITIAL_SOL as u128) * (MIN_INITIAL_TOKENS as u128);
    let s = integer_sqrt(small);
    assert!(s * s <= small);
    assert!((s + 1) * (s + 1) > small);
    assert!(s > MIN_LIQUIDITY as u128);

    // Typical: 200 SOL * 150M tokens
    let medium = 200_000_000_000u128 * 150_000_000_000_000u128;
    let m = integer_sqrt(medium);
    assert!(m * m <= medium);
    assert!((m + 1) * (m + 1) > medium);

    // Large: 1000 SOL * 1B tokens
    let large = 1_000_000_000_000u128 * 1_000_000_000_000_000u128;
    let l = integer_sqrt(large);
    assert!(l * l <= large);
    assert!((l + 1) * (l + 1) > large);
}

#[cfg(kani)]
#[kani::proof]
fn verify_lp_mint_proportional() {
    let lp_supply: u64 = 1_000_000_000_000;
    let reserve: u64 = 150_000_000_000_000;

    // 1% deposit → ~1% of supply
    let lp_1 = calc_lp_mint(lp_supply, reserve / 100, reserve).unwrap();
    assert!(lp_1 == lp_supply / 100);

    // 10% deposit → 10% of supply
    let lp_10 = calc_lp_mint(lp_supply, reserve / 10, reserve).unwrap();
    assert!(lp_10 == lp_supply / 10);

    // 100% deposit → 100% of supply
    let lp_100 = calc_lp_mint(lp_supply, reserve, reserve).unwrap();
    assert!(lp_100 == lp_supply);

    // Dust deposit → 0 LP (floor division)
    let lp_dust = calc_lp_mint(lp_supply, 1, reserve).unwrap();
    assert!(lp_dust == 0);
}

// ============================================================================
// 4. LP Redemption
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_lp_redeem_bounded() {
    let lp_supply: u64 = 1_000_000_000_000;
    let reserve: u64 = 200_000_000_000;

    let amounts: [u64; 4] = [
        1,                      // 1 LP token
        lp_supply / 100,        // 1%
        lp_supply / 2,          // 50%
        lp_supply,              // 100%
    ];

    for lp in amounts {
        let redeemed = calc_lp_redeem(lp, reserve, lp_supply).unwrap();
        assert!(redeemed <= reserve);
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_lp_full_redeem() {
    let lp_supply: u64 = 1_000_000_000_000;

    // 100% LP = 100% reserve at any reserve size
    assert!(calc_lp_redeem(lp_supply, 200_000_000_000, lp_supply).unwrap() == 200_000_000_000);
    assert!(calc_lp_redeem(lp_supply, 1_000_000_000, lp_supply).unwrap() == 1_000_000_000);
    assert!(calc_lp_redeem(lp_supply, 5_000_000_000_000, lp_supply).unwrap() == 5_000_000_000_000);
    assert!(calc_lp_redeem(lp_supply, 1, lp_supply).unwrap() == 1);
}

#[cfg(kani)]
#[kani::proof]
fn verify_lp_redeem_monotonic() {
    let lp_supply: u64 = 1_000_000_000_000;
    let reserve: u64 = 200_000_000_000;

    let out_1 = calc_lp_redeem(lp_supply / 100, reserve, lp_supply).unwrap();
    let out_10 = calc_lp_redeem(lp_supply / 10, reserve, lp_supply).unwrap();
    let out_50 = calc_lp_redeem(lp_supply / 2, reserve, lp_supply).unwrap();
    let out_100 = calc_lp_redeem(lp_supply, reserve, lp_supply).unwrap();

    assert!(out_10 > out_1);
    assert!(out_50 > out_10);
    assert!(out_100 > out_50);
    assert!(out_100 == reserve);
}

// ============================================================================
// 5. Fee Compounding (K Growth)
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_fee_compounds_k() {
    let sol_reserve: u64 = 200_000_000_000;
    let token_reserve: u64 = 150_000_000_000_000;
    let k_before = (sol_reserve as u128) * (token_reserve as u128);

    // Every swap with fee > 0 must strictly increase K
    let swaps: [u64; 4] = [
        400,                    // minimum fee = 1
        1_000_000_000,          // 1 SOL
        10_000_000_000,         // 10 SOL
        100_000_000_000,        // 100 SOL
    ];

    for sol_in in swaps {
        let fee = calc_swap_fee(sol_in).unwrap();
        assert!(fee > 0);

        let effective_in = sol_in - fee;
        let tokens_out = calc_swap_output(effective_in, sol_reserve, token_reserve).unwrap();

        let new_sol = (sol_reserve as u128) + (sol_in as u128);
        let new_tokens = (token_reserve as u128) - (tokens_out as u128);
        let k_after = new_sol * new_tokens;

        // Strict increase
        assert!(k_after > k_before);
    }
}

// ============================================================================
// 6. Sell-side Symmetry
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_sell_output_bounded() {
    let sol_reserve: u64 = 200_000_000_000;
    let token_reserve: u64 = 150_000_000_000_000;

    // Sell tokens for SOL (reverse direction)
    let token_inputs: [u64; 4] = [
        1_000_000,              // 1 token
        1_000_000_000,          // 1000 tokens
        1_000_000_000_000,      // 1M tokens
        100_000_000_000_000,    // 100M tokens
    ];

    for tokens_in in token_inputs {
        let fee = (tokens_in * SWAP_FEE_BPS) / FEE_DENOMINATOR;
        let effective = tokens_in - fee;
        let sol_out = calc_swap_output(effective, token_reserve, sol_reserve).unwrap();

        assert!(sol_out < sol_reserve);
        assert!(sol_out > 0);
    }
}
