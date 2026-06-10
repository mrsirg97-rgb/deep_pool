// Sanity tests — verify the harness can mint a Token-2022 token, create a
// pool, and read state.

use solana_sdk::{native_token::LAMPORTS_PER_SOL, signer::Signer};

use crate::harness::{create_mint, create_pool, get_pool, mint_to_user, Env};

#[test]
fn env_bootstraps() {
    let env = Env::new();
    assert!(env.balance(&env.payer.pubkey()) > 0);
}

#[test]
fn mint_and_pool_lifecycle() {
    let mut env = Env::new();
    let (mint, mint_authority) = create_mint(&mut env, 0, 6);

    let creator = env.new_funded(10 * LAMPORTS_PER_SOL);
    mint_to_user(
        &mut env,
        &mint,
        &mint_authority,
        &creator,
        1_000_000_000_000,
    ); // 1M tokens

    let pool_ctx = create_pool(
        &mut env,
        &creator,
        &mint,
        100_000_000_000,      // 100k tokens
        6 * LAMPORTS_PER_SOL, // 6 SOL (above the 5-SOL creation minimum)
    )
    .expect("create_pool failed");

    let pool = get_pool(&env, &pool_ctx.pool);
    assert_eq!(pool.config, creator.pubkey());
    assert_eq!(pool.token_mint, mint);
    assert_eq!(pool.initial_sol, 6 * LAMPORTS_PER_SOL);
    assert!(pool.initial_tokens > 0);
}
