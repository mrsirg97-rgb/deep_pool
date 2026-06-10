//! TWAP oracle integration. Proves the keeperless mechanism end-to-end: swaps
//! (and nothing else) advance the in-pool mark, and `read_twap_sol_per_tok`
//! returns a sane time-weighted price once the ring window rolls — `None` until
//! then (warmup → consumers fail closed). See docs/twap-oracle.md.

use crate::harness::*;
use deep_pool::constants::MIN_OBS_SPACING_SLOTS;
use deep_pool::math::price_q64;
use deep_pool::state::Pool;
use solana_sdk::{clock::Clock, native_token::LAMPORTS_PER_SOL, signer::Signer};

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

#[test]
fn ring_wraparound_preserves_max_lookback() {
    // [P-8] Drive >16 spaced swaps so the ring wraps. Reads within the ring's
    // surviving span still work; a lookback older than the surviving history
    // returns None (the oldest snapshots were overwritten).
    let mut env = Env::new();
    let p = fresh_pool(&mut env);
    let user = env.new_funded(50 * LAMPORTS_PER_SOL);

    let mut slot = now_slot(&env);
    for _ in 0..20 {
        slot += MIN_OBS_SPACING_SLOTS + 10;
        env.svm.warp_to_slot(slot);
        // 0.001-SOL buys: 20 of them drift price ~0.4% total, inside tolerance.
        swap(&mut env, &user, &p, 1_000_000, 1, true).expect("swap");
    }

    let pool = get_pool(&env, &p.pool);
    let (sol, tok) = reserves_now(&env, &p);
    let now = now_slot(&env);

    // Within the surviving span (16 snapshots ≈ 15 spacings back): readable.
    let mark = pool
        .read_twap_sol_per_tok(sol, tok, now, 10 * (MIN_OBS_SPACING_SLOTS + 10))
        .expect("mark within the ring span");
    let spot = price_q64(sol, tok).unwrap();
    let diff = if mark > spot { mark - spot } else { spot - mark };
    assert!(diff.saturating_mul(100) < spot, "post-wrap mark sane");

    // Beyond the surviving span: the overwritten history is gone → fail closed.
    assert!(
        pool
            .read_twap_sol_per_tok(sol, tok, now, 19 * (MIN_OBS_SPACING_SLOTS + 10))
            .is_none(),
        "lookback older than the ring's surviving span must be None"
    );
}

#[test]
fn dust_pool_advances_clock_without_accumulating() {
    // [P-8] Below MIN_SPOT_RESERVE the write path advances last_cum_slot but
    // does NOT accumulate — a thin pool's price never enters the cumulative,
    // and the skipped gap is not mis-weighted at the next above-floor swap.
    use deep_pool::constants::MIN_SPOT_RESERVE;

    let mut env = Env::new();
    // Build inline (fresh_pool drops the mint authority; the crash needs it).
    let (mint, authority) = create_mint(&mut env, 0, 6);
    let creator = env.new_funded(50 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &creator, 10_000_000_000_000);
    let p = create_pool(&mut env, &creator, &mint, 1_000_000_000_000, 10 * LAMPORTS_PER_SOL)
        .expect("create_pool");

    // Crash below the dust floor: whale dumps ~30x the token reserve.
    let whale = env.new_funded(2 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &whale, 60_000_000_000_000);
    swap(&mut env, &whale, &p, 30_000_000_000_000, 1, false).expect("crash sell");
    let (sol, _) = reserves_now(&env, &p);
    assert!(sol < MIN_SPOT_RESERVE, "pool must be sub-floor (got {sol})");

    // Sub-floor swap across a warp: clock advances, cumulative does not.
    let pool_before = get_pool(&env, &p.pool);
    let slot = now_slot(&env) + MIN_OBS_SPACING_SLOTS + 10;
    env.svm.warp_to_slot(slot);
    let user = env.new_funded(5 * LAMPORTS_PER_SOL);
    swap(&mut env, &user, &p, 10_000_000, 1, true).expect("sub-floor swap");
    let pool_after = get_pool(&env, &p.pool);
    assert_eq!(
        pool_after.cum_sol_per_tok, pool_before.cum_sol_per_tok,
        "sub-floor swap must not accumulate"
    );
    assert_eq!(pool_after.last_cum_slot, slot, "clock must advance");
}

#[test]
fn same_slot_swaps_accumulate_once() {
    // [P-8] record_observation is idempotent within a slot: the second swap in
    // the same slot sees now == last_cum_slot and leaves the cumulative alone.
    let mut env = Env::new();
    let p = fresh_pool(&mut env);
    let user = env.new_funded(50 * LAMPORTS_PER_SOL);

    let slot = now_slot(&env) + MIN_OBS_SPACING_SLOTS + 10;
    env.svm.warp_to_slot(slot);
    swap(&mut env, &user, &p, 10_000_000, 1, true).expect("swap 1");
    let cum_after_first = get_pool(&env, &p.pool).cum_sol_per_tok;
    swap(&mut env, &user, &p, 10_000_000, 1, true).expect("swap 2 (same slot)");
    let pool = get_pool(&env, &p.pool);
    assert_eq!(
        pool.cum_sol_per_tok, cum_after_first,
        "second same-slot swap must not re-accumulate"
    );
    assert_eq!(pool.last_cum_slot, slot);
}

#[test]
fn liquidity_ops_do_not_write_the_oracle() {
    // [P-8] add/remove move reserves (≈)proportionally — price-neutral — and
    // must not touch the oracle: no accumulation, no clock advance, no ring
    // write. The read's lazy head-extension covers the gap instead.
    use crate::harness::{add_liquidity, remove_liquidity};

    let mut env = Env::new();
    let p = fresh_pool(&mut env);
    let user = env.new_funded(50 * LAMPORTS_PER_SOL);
    // One swap so the oracle has non-trivial state.
    let slot = now_slot(&env) + MIN_OBS_SPACING_SLOTS + 10;
    env.svm.warp_to_slot(slot);
    swap(&mut env, &user, &p, 10_000_000, 1, true).expect("swap");

    // Provider adds + removes across another warp — oracle must be untouched.
    let provider = env.new_funded(50 * LAMPORTS_PER_SOL);
    // fresh_pool's creator holds the token supply; fund the provider via a buy.
    swap(&mut env, &provider, &p, LAMPORTS_PER_SOL, 1, true).expect("provider acquires tokens");
    let before2 = get_pool(&env, &p.pool); // (that swap may accumulate — re-baseline)
    env.svm.warp_to_slot(now_slot(&env) + MIN_OBS_SPACING_SLOTS + 10);
    add_liquidity(&mut env, &provider, &p, 50_000_000, 2 * LAMPORTS_PER_SOL, 1)
        .expect("add_liquidity");
    use crate::harness::derive_ata;
    let provider_lp = derive_ata(&provider.pubkey(), &p.lp_mint, &deep_pool::constants::TOKEN_2022_PROGRAM_ID);
    let lp_bal = get_token_amount(&env, &provider_lp);
    remove_liquidity(&mut env, &provider, &p, lp_bal / 2, 0, 0).expect("remove_liquidity");

    let after = get_pool(&env, &p.pool);
    assert_eq!(after.cum_sol_per_tok, before2.cum_sol_per_tok, "no accumulation");
    assert_eq!(after.last_cum_slot, before2.last_cum_slot, "no clock advance");
    assert_eq!(after.obs_head, before2.obs_head, "no ring write");
}
