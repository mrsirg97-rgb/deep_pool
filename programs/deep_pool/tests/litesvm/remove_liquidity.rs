// remove_liquidity tests — happy path, slippage, dust-floor enforcement (M1).

use solana_sdk::{native_token::LAMPORTS_PER_SOL, signature::Keypair, signer::Signer};

use crate::expect_err;
use crate::harness::{
    create_mint, create_pool, get_token_amount, mint_to_user, remove_liquidity, Env, PoolCtx,
};
use deep_pool::error::DeepPoolError;

fn setup_pool(env: &mut Env, fee_bps: u16) -> (PoolCtx, Keypair) {
    let (mint, authority) = create_mint(env, fee_bps, 6);
    let creator = env.new_funded(20 * LAMPORTS_PER_SOL);
    mint_to_user(env, &mint, &authority, &creator, 10_000_000_000_000);
    let p = create_pool(
        env,
        &creator,
        &mint,
        1_000_000_000_000,
        10 * LAMPORTS_PER_SOL,
    )
    .expect("create_pool");
    (p, creator)
}

#[test]
fn happy_path_partial_redeem() {
    let mut env = Env::new();
    let (p, creator) = setup_pool(&mut env, 0);

    use crate::harness::derive_ata;
    use deep_pool::constants::TOKEN_2022_PROGRAM_ID;
    let creator_lp = derive_ata(&creator.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let lp_balance = get_token_amount(&env, &creator_lp);
    assert!(lp_balance > 0);

    remove_liquidity(&mut env, &creator, &p, lp_balance / 4, 1, 1).expect("remove_liquidity");
}

#[test]
fn slippage_sol_rejected() {
    let mut env = Env::new();
    let (p, creator) = setup_pool(&mut env, 0);

    use crate::harness::derive_ata;
    use deep_pool::constants::TOKEN_2022_PROGRAM_ID;
    let creator_lp = derive_ata(&creator.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let lp_balance = get_token_amount(&env, &creator_lp);
    let r = remove_liquidity(&mut env, &creator, &p, lp_balance / 4, u64::MAX, 1);
    expect_err!(r, DeepPoolError::SolOutputSlippage);
}

#[test]
fn smallest_pool_full_redeem_leaves_locked_floor() {
    // [P-1] The headline LP-lock property, now true at the creation minimum:
    // a born-minimum pool (5 SOL / 5 tokens) lets the creator redeem ALL their
    // LP — the 20% locked LP keeps ~1 SOL / ~1 token in the pool, above the
    // 0.1-SOL / 1-token retention backstop. (Pre-split, the retention check
    // used the creation minimums and froze ALL LP in this exact pool.)
    use crate::harness::derive_ata;
    use deep_pool::constants::{
        MIN_INITIAL_SOL, MIN_INITIAL_TOKENS, MIN_POOL_RESERVE_SOL, MIN_POOL_RESERVE_TOKENS,
        TOKEN_2022_PROGRAM_ID,
    };
    use deep_pool::state::Pool;

    let mut env = Env::new();
    let (mint, authority) = create_mint(&mut env, 0, 6);
    let creator = env.new_funded(10 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &creator, 1_000_000_000);

    let p = create_pool(&mut env, &creator, &mint, MIN_INITIAL_TOKENS, MIN_INITIAL_SOL)
        .expect("create_pool at minimums");

    let creator_lp = derive_ata(&creator.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let lp_balance = get_token_amount(&env, &creator_lp);
    remove_liquidity(&mut env, &creator, &p, lp_balance, 1, 0)
        .expect("full creator redeem must succeed on a born-minimum pool");

    // Locked floor retained: ~20% of initial reserves, above the retention backstop.
    let rent = env.svm.minimum_balance_for_rent_exemption(Pool::LEN);
    let sol_remaining = env.svm.get_account(&p.pool).unwrap().lamports - rent;
    let tokens_remaining = get_token_amount(&env, &p.token_vault);
    assert!(
        sol_remaining >= MIN_POOL_RESERVE_SOL && sol_remaining >= MIN_INITIAL_SOL / 5 - 1_000,
        "locked LP keeps ~20% of SOL (got {sol_remaining})"
    );
    assert!(
        tokens_remaining >= MIN_POOL_RESERVE_TOKENS,
        "locked LP keeps ~20% of tokens (got {tokens_remaining})"
    );
}

#[test]
fn crashed_pool_retention_floor_blocks_last_exits() {
    // [P-1] The retention backstop's ONLY binding case: after a deep price
    // crash (sells draining sol_reserve), the locked-LP floor in SOL terms
    // falls below 0.1 SOL, and the backstop blocks the LAST dust of LP exits
    // (keeping the pool alive). A partial redeem that stays above the floor
    // still succeeds. Documented behavior — design.md "retention floor".
    use crate::harness::{derive_ata, swap};
    use deep_pool::constants::TOKEN_2022_PROGRAM_ID;
    use deep_pool::state::Pool;

    let mut env = Env::new();
    let (mint, authority) = create_mint(&mut env, 0, 6);
    let creator = env.new_funded(20 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &creator, 10_000_000_000_000);
    let p = create_pool(&mut env, &creator, &mint, 1_000_000_000_000, 5 * LAMPORTS_PER_SOL)
        .expect("create_pool");

    // Crash: a whale sells ~30x the token reserve, draining ~97% of pool SOL.
    let whale = env.new_funded(2 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &whale, 60_000_000_000_000);
    swap(&mut env, &whale, &p, 30_000_000_000_000, 1, false).expect("crash sell");

    let rent = env.svm.minimum_balance_for_rent_exemption(Pool::LEN);
    let sol_reserve = env.svm.get_account(&p.pool).unwrap().lamports - rent;
    assert!(
        sol_reserve < 500_000_000,
        "crash should leave < 0.5 SOL (got {sol_reserve})"
    );

    let creator_lp = derive_ata(&creator.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let lp_balance = get_token_amount(&env, &creator_lp);

    // Full redeem would leave 20% × crashed-reserve < 0.1 SOL → backstop fires.
    let r = remove_liquidity(&mut env, &creator, &p, lp_balance, 0, 0);
    expect_err!(r, DeepPoolError::MinimumLiquidityRequired);

    // A small redeem that leaves the pool above the backstop still works.
    remove_liquidity(&mut env, &creator, &p, lp_balance / 10, 0, 0)
        .expect("partial exit above the retention floor succeeds");
}

