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
fn dust_floor_rejected() {
    // M1: when the remaining SOL or tokens would fall below MIN_INITIAL_SOL /
    // MIN_INITIAL_TOKENS, the call must reject with MinimumLiquidityRequired.
    // On a pool created at exactly the protocol minimums, the 20% creator
    // lock keeps 20% of initial reserves permanently — but any partial
    // creator redeem still pushes us below the dust floor.
    use crate::harness::derive_ata;
    use deep_pool::constants::{MIN_INITIAL_SOL, MIN_INITIAL_TOKENS, TOKEN_2022_PROGRAM_ID};

    let mut env = Env::new();
    let (mint, authority) = create_mint(&mut env, 0, 6);
    let creator = env.new_funded(2 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &creator, 1_000_000_000);

    // Pool seeded at exactly the minimums — any creator-side redeem drops
    // remaining reserves below the floor.
    let p = create_pool(
        &mut env,
        &creator,
        &mint,
        MIN_INITIAL_TOKENS,
        MIN_INITIAL_SOL,
    )
    .expect("create_pool at minimums");

    let creator_lp = derive_ata(&creator.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let lp_balance = get_token_amount(&env, &creator_lp);
    let r = remove_liquidity(&mut env, &creator, &p, lp_balance / 2, 1, 1);
    expect_err!(r, DeepPoolError::MinimumLiquidityRequired);
}
