//! Property-based fuzz tests for `deep_pool::math`. Each `proptest!` block
//! runs the property across thousands of random inputs (default 256 cases;
//! bumped per block). Complements the Kani harnesses (exhaustive at concrete
//! values) by exploring the full u64 range with shrinking on failure.
//!
//! Located in `tests/` so the `proptest!` macro DSL (e.g. `amount in any::<u64>()`)
//! isn't parsed by anchor's `#[program]` safety-check macro, which walks the
//! lib source tree with syn and doesn't know about macro semantics.
//!
//! Run with `cargo test -p deep_pool --test math_proptests`.

use deep_pool::math::*;
use deep_pool::{FEE_DENOMINATOR, SWAP_FEE_BPS};
use proptest::prelude::*;

const RESERVE_MAX: u64 = 1_000_000_000_000_000_000;

// ============================================================================
// calc_swap_fee
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn swap_fee_never_panics_and_is_bounded(amount in any::<u64>()) {
        if let Some(f) = calc_swap_fee(amount) {
            prop_assert!(f <= amount);
            prop_assert_eq!(
                f,
                (amount as u128 * SWAP_FEE_BPS as u128 / FEE_DENOMINATOR as u128) as u64,
            );
        }
    }

    #[test]
    fn swap_fee_monotonic(a in 0u64..u64::MAX/100, b in 0u64..u64::MAX/100) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let fa = calc_swap_fee(lo).unwrap();
        let fb = calc_swap_fee(hi).unwrap();
        prop_assert!(fb >= fa);
    }

    #[test]
    fn swap_fee_conservation(amount in 0u64..u64::MAX/100) {
        let fee = calc_swap_fee(amount).unwrap();
        let effective = amount - fee;
        prop_assert_eq!(fee + effective, amount);
    }
}

// ============================================================================
// calc_swap_output — constant-product curve
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn swap_output_bounded_by_reserve(
        effective_in in 1u64..RESERVE_MAX,
        input_reserve in 1u64..RESERVE_MAX,
        output_reserve in 1u64..RESERVE_MAX,
    ) {
        let output = calc_swap_output(effective_in, input_reserve, output_reserve).unwrap();
        prop_assert!(output < output_reserve);
    }

    #[test]
    fn swap_output_zero_input_is_zero(
        input_reserve in 1u64..RESERVE_MAX,
        output_reserve in 1u64..RESERVE_MAX,
    ) {
        prop_assert_eq!(calc_swap_output(0, input_reserve, output_reserve).unwrap(), 0);
    }

    #[test]
    fn swap_output_monotonic_in_input(
        a in 1u64..RESERVE_MAX / 2,
        delta in 1u64..RESERVE_MAX / 2,
        input_reserve in 1u64..RESERVE_MAX,
        output_reserve in 1u64..RESERVE_MAX,
    ) {
        let b = a.saturating_add(delta);
        let out_a = calc_swap_output(a, input_reserve, output_reserve).unwrap();
        let out_b = calc_swap_output(b, input_reserve, output_reserve).unwrap();
        prop_assert!(out_b >= out_a);
    }

    #[test]
    fn swap_k_non_decreasing(
        amount_in in 1u64..100_000_000_000_000,
        input_reserve in 1_000_000_000u64..100_000_000_000_000,
        output_reserve in 1_000_000_000u64..1_000_000_000_000_000_000,
    ) {
        let fee = calc_swap_fee(amount_in).unwrap();
        let effective_in = amount_in - fee;
        let out = calc_swap_output(effective_in, input_reserve, output_reserve).unwrap();

        let k_before = (input_reserve as u128) * (output_reserve as u128);
        let new_input = (input_reserve as u128) + (amount_in as u128);
        let new_output = (output_reserve as u128) - (out as u128);
        let k_after = new_input * new_output;

        prop_assert!(k_after >= k_before, "k decreased: {} -> {}", k_before, k_after);
    }
}

// ============================================================================
// calc_lp_mint
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn lp_mint_zero_deposit_is_zero(
        lp_supply in 1u64..RESERVE_MAX,
        reserve in 1u64..RESERVE_MAX,
    ) {
        prop_assert_eq!(calc_lp_mint(lp_supply, 0, reserve).unwrap(), 0);
    }

    #[test]
    fn lp_mint_full_deposit_equals_supply(
        lp_supply in 1u64..RESERVE_MAX,
        reserve in 1u64..RESERVE_MAX,
    ) {
        prop_assert_eq!(calc_lp_mint(lp_supply, reserve, reserve).unwrap(), lp_supply);
    }

    #[test]
    fn lp_mint_monotonic_in_deposit(
        lp_supply in 1u64..1_000_000_000_000,
        reserve in 1u64..1_000_000_000_000,
        a in 0u64..1_000_000_000,
        b in 0u64..1_000_000_000,
    ) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let la = calc_lp_mint(lp_supply, lo, reserve).unwrap();
        let lb = calc_lp_mint(lp_supply, hi, reserve).unwrap();
        prop_assert!(lb >= la);
    }

    #[test]
    fn lp_mint_bounded_when_deposit_le_reserve(
        lp_supply in 1u64..RESERVE_MAX,
        reserve in 1u64..RESERVE_MAX,
        deposit_frac in 0u64..=10_000u64,
    ) {
        let deposit = ((reserve as u128 * deposit_frac as u128) / 10_000) as u64;
        let minted = calc_lp_mint(lp_supply, deposit, reserve).unwrap();
        prop_assert!(minted <= lp_supply);
    }
}

// ============================================================================
// calc_lp_redeem
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn lp_redeem_zero_is_zero(
        reserve in 1u64..RESERVE_MAX,
        lp_supply in 1u64..RESERVE_MAX,
    ) {
        prop_assert_eq!(calc_lp_redeem(0, reserve, lp_supply).unwrap(), 0);
    }

    #[test]
    fn lp_redeem_full_supply_equals_reserve(
        reserve in 1u64..RESERVE_MAX,
        lp_supply in 1u64..RESERVE_MAX,
    ) {
        prop_assert_eq!(calc_lp_redeem(lp_supply, reserve, lp_supply).unwrap(), reserve);
    }

    #[test]
    fn lp_redeem_bounded_by_reserve(
        reserve in 1u64..RESERVE_MAX,
        lp_supply in 1u64..RESERVE_MAX,
        lp_amount_frac in 0u64..=10_000u64,
    ) {
        let lp_amount = ((lp_supply as u128 * lp_amount_frac as u128) / 10_000) as u64;
        let out = calc_lp_redeem(lp_amount, reserve, lp_supply).unwrap();
        prop_assert!(out <= reserve);
    }

    #[test]
    fn lp_mint_redeem_roundtrip_no_extraction(
        lp_supply in 1_000_000u64..1_000_000_000_000,
        reserve in 1_000_000u64..1_000_000_000_000,
        deposit_frac in 1u64..10_000u64,
    ) {
        let deposit = ((reserve as u128 * deposit_frac as u128) / 10_000) as u64;
        prop_assume!(deposit > 0);
        let minted = calc_lp_mint(lp_supply, deposit, reserve).unwrap();
        prop_assume!(minted > 0);

        let new_supply = lp_supply + minted;
        let new_reserve = reserve + deposit;
        let redeemed = calc_lp_redeem(minted, new_reserve, new_supply).unwrap();

        prop_assert!(
            redeemed <= deposit,
            "roundtrip extracted value: deposit={} redeemed={}",
            deposit, redeemed,
        );
    }
}

// ============================================================================
// integer_sqrt
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn integer_sqrt_is_floor(n in 0u128..u64::MAX as u128) {
        let s = integer_sqrt(n);
        prop_assert!(s.checked_mul(s).unwrap() <= n);
        let sp1 = s + 1;
        prop_assert!(sp1.checked_mul(sp1).unwrap() > n);
    }

    #[test]
    fn integer_sqrt_monotonic(a in 0u128..u64::MAX as u128, b in 0u128..u64::MAX as u128) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(integer_sqrt(hi) >= integer_sqrt(lo));
    }

    #[test]
    fn integer_sqrt_perfect_squares(s in 0u64..u32::MAX as u64) {
        let n = (s as u128) * (s as u128);
        prop_assert_eq!(integer_sqrt(n), s as u128);
    }
}

// ============================================================================
// Composite: swap roundtrip cannot extract value
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5_000))]

    #[test]
    fn swap_roundtrip_no_extraction(
        sol_in in 1_000_000u64..10_000_000_000_000,
        sol_reserve in 1_000_000_000u64..100_000_000_000_000,
        token_reserve in 1_000_000_000u64..10_000_000_000_000_000,
    ) {
        let fee_in = calc_swap_fee(sol_in).unwrap();
        let eff_in = sol_in - fee_in;
        let tokens_out = calc_swap_output(eff_in, sol_reserve, token_reserve).unwrap();
        prop_assume!(tokens_out > 0);

        let new_sol_r = sol_reserve + sol_in;
        let new_tok_r = token_reserve - tokens_out;

        let fee_mid = match calc_swap_fee(tokens_out) {
            Some(f) => f,
            None => return Ok(()),
        };
        let eff_mid = tokens_out - fee_mid;
        let sol_back = calc_swap_output(eff_mid, new_tok_r, new_sol_r).unwrap();

        prop_assert!(
            sol_back <= sol_in,
            "roundtrip profit: sol_in={} sol_back={}",
            sol_in, sol_back,
        );
    }
}
