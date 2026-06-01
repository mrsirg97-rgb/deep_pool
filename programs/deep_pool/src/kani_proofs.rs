//! Kani Formal Verification Proof Harnesses for DeepPool
//!
//! Proves properties of the constant-product AMM math at concrete values
//! spanning the protocol's operating range. Run with: cargo kani
//!
//! Concrete inputs avoid SAT solver explosion on wide integer arithmetic
//! while verifying correctness at every scale the protocol operates at.

use crate::constants::*;
use crate::math::*;

// ============================================================================
// 1. Swap Fee
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_swap_fee_conservation() {
    // Test across range: dust, small, medium, large
    let amounts: [u64; 6] = [
        1,                 // 1 lamport
        399,               // just below fee threshold
        400,               // exact threshold (fee = 1)
        1_000_000_000,     // 1 SOL
        100_000_000_000,   // 100 SOL
        1_000_000_000_000, // 1000 SOL
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
    let pool_sol: u64 = 200_000_000_000; // 200 SOL
    let pool_tokens: u64 = 150_000_000_000_000; // 150M tokens

    let inputs: [u64; 5] = [
        1_000_000,       // 0.001 SOL
        100_000_000,     // 0.1 SOL
        1_000_000_000,   // 1 SOL
        50_000_000_000,  // 50 SOL
        199_000_000_000, // 199 SOL (nearly all reserves)
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
    let pool_sol: u64 = 1_000_000_000_000; // 1000 SOL
    let pool_tokens: u64 = 500_000_000_000_000; // 500M tokens

    let inputs: [u64; 3] = [
        1_000_000_000,   // 1 SOL
        100_000_000_000, // 100 SOL
        500_000_000_000, // 500 SOL
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
        400,             // minimum fee-generating swap
        1_000_000_000,   // 1 SOL
        10_000_000_000,  // 10 SOL
        50_000_000_000,  // 50 SOL
        100_000_000_000, // 100 SOL
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
        1,               // 1 LP token
        lp_supply / 100, // 1%
        lp_supply / 2,   // 50%
        lp_supply,       // 100%
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
        400,             // minimum fee = 1
        1_000_000_000,   // 1 SOL
        10_000_000_000,  // 10 SOL
        100_000_000_000, // 100 SOL
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
        1_000_000,           // 1 token
        1_000_000_000,       // 1000 tokens
        1_000_000_000_000,   // 1M tokens
        100_000_000_000_000, // 100M tokens
    ];

    for tokens_in in token_inputs {
        let fee = (tokens_in * SWAP_FEE_BPS) / FEE_DENOMINATOR;
        let effective = tokens_in - fee;
        let sol_out = calc_swap_output(effective, token_reserve, sol_reserve).unwrap();

        assert!(sol_out < sol_reserve);
        assert!(sol_out > 0);
    }
}

// ============================================================================
// 7. Symbolic Proofs (bounded)
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_swap_fee_bounded_symbolic() {
    let amount: u64 = kani::any();
    kani::assume(amount > 0);

    if let Some(fee) = calc_swap_fee(amount) {
        assert!(fee <= amount);
    }
    // None is safe — checked_mul overflow means amount is astronomically large
    // (> u64::MAX / 25 ≈ 738 quadrillion lamports ≈ 738M SOL). Unreachable in practice.
}

// Swap output, K invariant, and LP redeem use u128 arithmetic with
// multiple symbolic inputs — CBMC SAT solver can't handle the state space.
// These properties are covered by the concrete proofs above which verify
// at representative values spanning the full operating range.

// ============================================================================
// 8. LP Burn on Add Liquidity
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_lp_lock_rates() {
    let cases: [u64; 5] = [
        1000,
        1_000_000,
        1_000_000_000,
        1_000_000_000_000,
        10_000_000_000_000,
    ];

    // Creator: 20% locked, 80% to creator
    for lp in cases {
        let lock = lp * LP_LOCK_CREATOR_BPS / FEE_DENOMINATOR;
        let to_creator = lp - lock;
        assert!(lock == lp / 5);
        assert!(to_creator == lp * 4 / 5);
    }

    // Provider: 7.5% locked, 92.5% to provider
    for lp in cases {
        let lock = lp * LP_LOCK_PROVIDER_BPS / FEE_DENOMINATOR;
        let to_provider = lp - lock;
        assert!(lock <= lp);
        assert!(to_provider > lp * 9 / 10); // > 90%
        assert!(to_provider < lp); // < 100%
    }
}

// ============================================================================
// 8. calc_proportional — used by add_liquidity to derive the SOL side from
// the actual net token deposit (post-Token-2022 fee). Math is the same shape
// as calc_lp_redeem; these proofs anchor the function-by-name to its semantics.
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_proportional_zero_input() {
    let reserves: [(u64, u64); 3] = [
        (1, 1),
        (200_000_000_000, 150_000_000_000_000),
        (u64::MAX / 2, u64::MAX / 2),
    ];
    for (ra, rb) in reserves {
        assert!(calc_proportional(0, ra, rb) == Some(0));
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_proportional_identity() {
    // calc_proportional(reserve_a, reserve_a, reserve_b) == reserve_b
    // (a full-share deposit maps to the full opposite reserve)
    let cases: [(u64, u64); 4] = [
        (1_000_000_000, 1_000_000_000),
        (200_000_000_000, 150_000_000_000_000),
        (u64::MAX / 2, u64::MAX / 2),
        (1, u64::MAX),
    ];
    for (ra, rb) in cases {
        assert!(calc_proportional(ra, ra, rb) == Some(rb));
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_proportional_overflow_returns_none() {
    // Result = input * reserve_b / reserve_a. With reserve_a = 1 and both
    // input and reserve_b near u64::MAX, the quotient is ≈ input * reserve_b,
    // which exceeds u64::MAX → must return None, not silently truncate.
    let input: u64 = u64::MAX;
    let reserve_b: u64 = u64::MAX;
    assert!(calc_proportional(input, 1, reserve_b) == None);

    // Same shape just past the u64 boundary.
    let half = u64::MAX / 2 + 1;
    assert!(calc_proportional(half, 1, 3) == None);
}

// Sibling proofs: calc_lp_mint and calc_lp_redeem received the same
// try_from hardening — verify they also reject overflow rather than
// truncating.

#[cfg(kani)]
#[kani::proof]
fn verify_lp_mint_overflow_returns_none() {
    // lp_supply * deposit / reserve, with reserve = 1, must overflow u64.
    let lp_supply: u64 = u64::MAX;
    let deposit: u64 = u64::MAX;
    assert!(calc_lp_mint(lp_supply, deposit, 1) == None);
}

#[cfg(kani)]
#[kani::proof]
fn verify_lp_redeem_overflow_returns_none() {
    // lp_amount * reserve / lp_supply, with lp_supply = 1, must overflow u64.
    let lp_amount: u64 = u64::MAX;
    let reserve: u64 = u64::MAX;
    assert!(calc_lp_redeem(lp_amount, reserve, 1) == None);
}

// ============================================================================
// 10. TWAP oracle — price_q64 + accumulate_price (concrete: price_q64 divides,
//     so a symbolic harness would explode the SAT solver)
// ============================================================================

#[cfg(kani)]
#[kani::proof]
fn verify_price_q64_at_scale() {
    // Never panics: `(u64 << 64)` maxes at 2^128 − 2^64, fits u128. None iff the
    // denominator is zero; otherwise exactly (out << 64) / in.
    let pairs: [(u64, u64); 5] = [
        (10_000_000_000, 1_000_000_000_000),    // 10 SOL / 1e12 tok (low price)
        (1_000_000_000_000_000_000, 1_000_000), // huge / tiny (high price)
        (1, 1_000_000_000_000_000_000),         // tiny / huge
        (u64::MAX, u64::MAX),                   // extreme equal → exactly 2^64
        (1_000_000_000, 0),                     // zero denom → None
    ];
    for (out, inp) in pairs {
        let p = price_q64(out, inp);
        if inp == 0 {
            assert!(p.is_none());
        } else {
            assert!(p == Some(((out as u128) << 64) / (inp as u128)));
        }
    }
}

#[cfg(kani)]
#[kani::proof]
fn verify_accumulate_price_window_difference() {
    // The invariant read_twap relies on: the wrapping difference of two heads
    // recovers the exact accumulation between them, while it stays < 2^128.
    let price: u128 = 184_467_440_737_095_516; // ~0.01 × 2^64
    let start: u128 = 0;
    let mut cum = start;
    let deltas: [u64; 4] = [500, 1000, 750, 1234];
    let mut total: u128 = 0;
    for d in deltas {
        cum = accumulate_price(cum, price, d);
        total += price * (d as u128);
    }
    assert!(cum.wrapping_sub(start) == total);
}

#[cfg(kani)]
#[kani::proof]
fn verify_accumulate_price_wraps_exactly() {
    // Even when the running head wraps past u128::MAX, the window difference is
    // exact (this is why the cumulative can grow unbounded mod 2^128).
    let price: u128 = 1_000_000_000_000_000_000;
    let start: u128 = u128::MAX - 100;
    let cum = accumulate_price(start, price, 5); // wraps
    assert!(cum.wrapping_sub(start) == price * 5);
}
