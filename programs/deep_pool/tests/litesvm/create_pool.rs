// create_pool tests — happy path + error variants + Token-2022 fee handling.

use solana_sdk::{
    native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair, signer::Signer,
};

use crate::expect_err;
use crate::harness::{create_mint, create_pool, get_pool, mint_to_user, Env};
use deep_pool::constants::*;
use deep_pool::error::DeepPoolError;

/// Standard setup: vanilla 0-fee Token-2022 mint + funded creator with tokens.
fn setup(env: &mut Env, fee_bps: u16) -> (Pubkey, Keypair) {
    let (mint, authority) = create_mint(env, fee_bps, 6);
    let creator = env.new_funded(10 * LAMPORTS_PER_SOL);
    mint_to_user(env, &mint, &authority, &creator, 10_000_000_000_000);
    (mint, creator)
}

#[test]
fn happy_path() {
    let mut env = Env::new();
    let (mint, creator) = setup(&mut env, 0);
    let p = create_pool(
        &mut env,
        &creator,
        &mint,
        100_000_000_000,
        2 * LAMPORTS_PER_SOL,
    )
    .expect("create_pool");

    let pool = get_pool(&env, &p.pool);
    assert_eq!(pool.config, creator.pubkey());
    assert_eq!(pool.token_mint, mint);
    assert_eq!(pool.initial_sol, 2 * LAMPORTS_PER_SOL);
    assert!(pool.initial_tokens > 0);
}

#[test]
fn rejects_initial_sol_below_minimum() {
    let mut env = Env::new();
    let (mint, creator) = setup(&mut env, 0);
    let r = create_pool(
        &mut env,
        &creator,
        &mint,
        100_000_000_000,
        MIN_INITIAL_SOL - 1,
    );
    expect_err!(r, DeepPoolError::InsufficientInitialSol);
}

#[test]
fn rejects_initial_tokens_below_minimum() {
    let mut env = Env::new();
    let (mint, creator) = setup(&mut env, 0);
    let r = create_pool(
        &mut env,
        &creator,
        &mint,
        MIN_INITIAL_TOKENS - 1,
        2 * LAMPORTS_PER_SOL,
    );
    expect_err!(r, DeepPoolError::InsufficientInitialTokens);
}

#[test]
fn token_2022_fee_lands_net_in_vault() {
    // With a 100 bps (1%) transfer fee, the vault receives ~99% of the gross
    // deposit. The handler measures net via vault reload and seeds LP supply
    // from sqrt(sol * net_tokens), so pool.initial_tokens must be strictly
    // less than the requested gross.
    let mut env = Env::new();
    let (mint, creator) = setup(&mut env, 100);
    let p = create_pool(
        &mut env,
        &creator,
        &mint,
        100_000_000_000,
        2 * LAMPORTS_PER_SOL,
    )
    .expect("create_pool");
    let pool = get_pool(&env, &p.pool);
    assert!(pool.initial_tokens < 100_000_000_000); // net < gross
    assert!(pool.initial_tokens >= 99_000_000_000); // ~1% loss
}
