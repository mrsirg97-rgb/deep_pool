//! TWAP oracle integration. Proves the keeperless mechanism end-to-end: swaps
//! (and nothing else) advance the in-pool mark, and `read_twap_sol_per_tok`
//! returns a sane time-weighted price once the ring window rolls — `None` until
//! then (warmup → consumers fail closed). See docs/twap-oracle.md.

use crate::harness::*;
use deep_pool::constants::MIN_OBS_SPACING_SLOTS;
use deep_pool::math::price_q64;
use deep_pool::state::Pool;
use solana_sdk::{clock::Clock, native_token::LAMPORTS_PER_SOL};

// 10 SOL / 1e12 tokens, 0 transfer fee — above MIN_SPOT_RESERVE (5 SOL).
fn fresh_pool(env: &mut Env) -> PoolCtx {
    let (mint, authority) = create_mint(env, 0, 6);
    let creator = env.new_funded(50 * LAMPORTS_PER_SOL);
    mint_to_user(env, &mint, &authority, &creator, 10_000_000_000_000);
    create_pool(env, &creator, &mint, 1_000_000_000_000, 10 * LAMPORTS_PER_SOL)
        .expect("create_pool")
}

fn now_slot(env: &Env) -> u64 {
    env.svm.get_sysvar::<Clock>().slot
}

// sol_reserve = pool lamports − rent-exempt minimum (mirrors Pool::sol_reserve).
fn reserves_now(env: &Env, p: &PoolCtx) -> (u64, u64) {
    let rent = env.svm.minimum_balance_for_rent_exemption(Pool::LEN);
    let sol = env.svm.get_account(&p.pool).unwrap().lamports - rent;
    let tok = get_token_amount(env, &p.token_vault);
    (sol, tok)
}

#[test]
fn warmup_mark_is_none_before_window() {
    let mut env = Env::new();
    let p = fresh_pool(&mut env);

    // No swaps yet → no observations → mark unavailable (fail-closed).
    let pool = get_pool(&env, &p.pool);
    let (sol, tok) = reserves_now(&env, &p);
    assert!(
        pool
            .read_twap_sol_per_tok(sol, tok, now_slot(&env), MIN_OBS_SPACING_SLOTS)
            .is_none(),
        "mark must be None during warmup"
    );
}

#[test]
fn twap_tracks_price_after_window_no_keeper() {
    let mut env = Env::new();
    let p = fresh_pool(&mut env);
    let user = env.new_funded(50 * LAMPORTS_PER_SOL);

    // Drive swaps spaced > MIN_OBS_SPACING apart so the ring fills with real
    // history. Tiny buys (0.01 SOL on a 10 SOL pool) keep price ~constant, so the
    // time-weighted mark should land within a hair of spot. No crank anywhere.
    let mut slot = now_slot(&env);
    for _ in 0..4 {
        slot += MIN_OBS_SPACING_SLOTS + 10;
        env.svm.warp_to_slot(slot);
        swap(&mut env, &user, &p, 10_000_000, 1, true).expect("swap");
    }

    let pool = get_pool(&env, &p.pool);
    let (sol, tok) = reserves_now(&env, &p);
    let now = now_slot(&env);
    let mark = pool
        .read_twap_sol_per_tok(sol, tok, now, MIN_OBS_SPACING_SLOTS)
        .expect("mark available after the window rolls");

    let spot = price_q64(sol, tok).unwrap();
    let diff = if mark > spot { mark - spot } else { spot - mark };
    assert!(
        diff.saturating_mul(100) < spot, // within 1%
        "twap mark {} should be within 1% of spot {} (diff {})",
        mark,
        spot,
        diff
    );
}
