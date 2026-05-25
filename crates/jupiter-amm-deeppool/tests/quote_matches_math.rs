//! Quote-math regression tests.
//!
//! Asserts that `DeepPoolAmm::quote` produces the same `out_amount` as a
//! direct call into `deep_pool::math` — i.e. the trait wrapper does not
//! drift from the on-chain swap math. Covers:
//!
//!   1. Buy (SOL → Token), no transfer fee
//!   2. Sell (Token → SOL), no transfer fee
//!   3. Buy with Token-2022 transfer fee on the output leg
//!   4. Sell with Token-2022 transfer fee on the input leg
//!   5. ExactOut rejected
//!   6. Zero-input rejected
//!   7. Wrong mint pair rejected
//!   8. Pre-`update` AMM reports inactive
//!
//! Real-account roundtrip (DeepPoolAmm fed by litesvm-generated state, then
//! comparing the predicted out_amount against the actual on-chain
//! `SwapExecuted` event) is a follow-up; see crate README.

use jupiter_amm_interface::{Amm, QuoteParams, SwapMode};
use solana_pubkey::{pubkey, Pubkey};

use deep_pool::math::{calc_swap_fee, calc_swap_output};
use jupiter_amm_deeppool::DeepPoolAmm;

const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const PROGRAM_ID: Pubkey = pubkey!("CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT");
const POOL_KEY: Pubkey = pubkey!("Po11111111111111111111111111111111111111112");
const TOKEN_MINT: Pubkey = pubkey!("To11111111111111111111111111111111111111112");
const TOKEN_VAULT: Pubkey = pubkey!("Va11111111111111111111111111111111111111112");

const SOL_RESERVE: u64 = 100_000_000_000; // 100 SOL
const TOKEN_RESERVE: u64 = 10_000_000_000_000; // 10M tokens @ 6 decimals

fn amm(transfer_fee_bps: u16) -> DeepPoolAmm {
    DeepPoolAmm::new_for_test(
        PROGRAM_ID,
        POOL_KEY,
        TOKEN_MINT,
        TOKEN_VAULT,
        SOL_RESERVE,
        TOKEN_RESERVE,
        transfer_fee_bps,
        u64::MAX,
    )
}

/// Token-2022 transfer-fee calculation — **ceiling** division, matching
/// spl-token-2022's `TransferFeeConfig::calculate_fee`. Used in both
/// directions of the predict helpers so they encode the same truth as
/// the on-chain transfer path.
fn xfer_fee_ceil(amount: u64, fee_bps: u16) -> u64 {
    if fee_bps == 0 || amount == 0 {
        return 0;
    }
    let n = (amount as u128) * (fee_bps as u128);
    ((n + 9_999u128) / 10_000u128) as u64
}

/// Re-derive the buy output via the on-chain math functions directly. The
/// AMM crate's `quote()` must produce the same number — otherwise the trait
/// wrapper has drifted from the program.
fn predict_buy(amount_in: u64, fee_bps: u16) -> u64 {
    let pool_fee = calc_swap_fee(amount_in).unwrap();
    let effective = amount_in - pool_fee;
    let gross_out = calc_swap_output(effective, SOL_RESERVE, TOKEN_RESERVE).unwrap();
    gross_out - xfer_fee_ceil(gross_out, fee_bps)
}

fn predict_sell(amount_in: u64, fee_bps: u16) -> u64 {
    let net_in = amount_in - xfer_fee_ceil(amount_in, fee_bps);
    let pool_fee = calc_swap_fee(net_in).unwrap();
    let effective = net_in - pool_fee;
    calc_swap_output(effective, TOKEN_RESERVE, SOL_RESERVE).unwrap()
}

fn qparams(amount: u64, input: Pubkey, output: Pubkey, mode: SwapMode) -> QuoteParams {
    QuoteParams {
        amount,
        input_mint: input,
        output_mint: output,
        swap_mode: mode,
        fee_mode: Default::default(),
    }
}

#[test]
fn buy_no_fee_matches_math() {
    let amm = amm(0);
    let amount = 1_000_000_000; // 1 SOL
    let q = amm
        .quote(&qparams(amount, WSOL_MINT, TOKEN_MINT, SwapMode::ExactIn))
        .expect("quote");
    assert_eq!(q.in_amount, amount);
    assert_eq!(q.out_amount, predict_buy(amount, 0));
    assert_eq!(q.fee_mint, WSOL_MINT, "buy fee denominated in SOL");
    assert!(q.fee_amount > 0);
}

#[test]
fn sell_no_fee_matches_math() {
    let amm = amm(0);
    let amount = 1_000_000_000_000; // 1M tokens
    let q = amm
        .quote(&qparams(amount, TOKEN_MINT, WSOL_MINT, SwapMode::ExactIn))
        .expect("quote");
    assert_eq!(q.in_amount, amount);
    assert_eq!(q.out_amount, predict_sell(amount, 0));
    assert_eq!(q.fee_mint, TOKEN_MINT, "sell fee denominated in token");
    assert!(q.fee_amount > 0);
}

#[test]
fn buy_with_transfer_fee_matches_math() {
    let fee_bps = 100; // 1%
    let amm = amm(fee_bps);
    let amount = 1_000_000_000;
    let q = amm
        .quote(&qparams(amount, WSOL_MINT, TOKEN_MINT, SwapMode::ExactIn))
        .expect("quote");
    assert_eq!(q.out_amount, predict_buy(amount, fee_bps));
    assert!(
        q.out_amount < predict_buy(amount, 0),
        "transfer fee must reduce output"
    );
}

#[test]
fn sell_with_transfer_fee_matches_math() {
    let fee_bps = 100; // 1%
    let amm = amm(fee_bps);
    let amount = 1_000_000_000_000;
    let q = amm
        .quote(&qparams(amount, TOKEN_MINT, WSOL_MINT, SwapMode::ExactIn))
        .expect("quote");
    assert_eq!(q.out_amount, predict_sell(amount, fee_bps));
    assert!(
        q.out_amount < predict_sell(amount, 0),
        "transfer fee must reduce output"
    );
}

#[test]
fn exact_out_rejected() {
    let amm = amm(0);
    let r = amm.quote(&qparams(
        1_000_000_000,
        WSOL_MINT,
        TOKEN_MINT,
        SwapMode::ExactOut,
    ));
    assert!(r.is_err(), "ExactOut not yet supported");
}

#[test]
fn zero_input_rejected() {
    let amm = amm(0);
    let r = amm.quote(&qparams(0, WSOL_MINT, TOKEN_MINT, SwapMode::ExactIn));
    assert!(r.is_err());
}

#[test]
fn wrong_mint_pair_rejected() {
    let amm = amm(0);
    // Two arbitrary pubkeys distinct from both WSOL_MINT and TOKEN_MINT.
    let other_a = Pubkey::new_from_array([42u8; 32]);
    let other_b = Pubkey::new_from_array([99u8; 32]);
    let r = amm.quote(&qparams(1_000_000, other_a, other_b, SwapMode::ExactIn));
    assert!(r.is_err());
}

#[test]
fn pre_update_amm_is_inactive() {
    // is_active() reflects "has fresh reserves to quote against". A
    // freshly-constructed (zero-reserve) AMM must report false so Jupiter's
    // router skips it until after the first `update`.
    let empty = DeepPoolAmm::new_for_test(
        PROGRAM_ID,
        POOL_KEY,
        TOKEN_MINT,
        TOKEN_VAULT,
        0, // sol_reserve = 0
        0, // token_reserve = 0
        0,
        u64::MAX,
    );
    assert!(!empty.is_active());

    let live = amm(0);
    assert!(live.is_active());
}

#[test]
fn trait_shape() {
    let amm = amm(0);
    assert_eq!(amm.label(), "DeepPool");
    assert_eq!(amm.program_id(), PROGRAM_ID);
    assert_eq!(amm.key(), POOL_KEY);
    assert_eq!(amm.get_reserve_mints(), vec![WSOL_MINT, TOKEN_MINT]);
    assert_eq!(amm.get_accounts_to_update(), vec![POOL_KEY, TOKEN_VAULT, TOKEN_MINT]);
    assert_eq!(amm.get_accounts_len(), 10);
    // clone_amm returns a boxed clone — must not panic.
    let _clone = amm.clone_amm();
}
