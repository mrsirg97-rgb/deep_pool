// swap tests — buy + sell paths, slippage, k-invariant growth.

use solana_sdk::{
    native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair, signer::Signer,
};

use crate::expect_err;
use crate::harness::{
    create_mint, create_pool, get_token_amount, mint_to_user, swap, Env, PoolCtx,
};
use deep_pool::error::DeepPoolError;

fn live_pool(env: &mut Env, fee_bps: u16) -> (PoolCtx, Pubkey, Keypair) {
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
    (p, mint, authority)
}

#[test]
fn buy_happy_path() {
    let mut env = Env::new();
    let (p, _mint, _authority) = live_pool(&mut env, 0);
    let user = env.new_funded(5 * LAMPORTS_PER_SOL);

    use crate::harness::derive_ata;
    use deep_pool::constants::TOKEN_2022_PROGRAM_ID;

    swap(&mut env, &user, &p, 1 * LAMPORTS_PER_SOL, 1, true).expect("buy");

    let user_token = derive_ata(&user.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    assert!(get_token_amount(&env, &user_token) > 0);
}

#[test]
fn sell_happy_path() {
    let mut env = Env::new();
    let (p, mint, authority) = live_pool(&mut env, 0);
    let user = env.new_funded(2 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &user, 100_000_000_000);

    let user_sol_before = env.balance(&user.pubkey());
    swap(&mut env, &user, &p, 50_000_000_000, 1, false).expect("sell");
    let user_sol_after = env.balance(&user.pubkey());
    assert!(user_sol_after > user_sol_before);
}

#[test]
fn buy_slippage_rejected() {
    let mut env = Env::new();
    let (p, _mint, _authority) = live_pool(&mut env, 0);
    let user = env.new_funded(5 * LAMPORTS_PER_SOL);
    // minimum_out = u64::MAX is unreachable — pool only has ~1T tokens
    let r = swap(&mut env, &user, &p, 1 * LAMPORTS_PER_SOL, u64::MAX, true);
    expect_err!(r, DeepPoolError::SlippageExceeded);
}

#[test]
fn sell_slippage_rejected() {
    let mut env = Env::new();
    let (p, mint, authority) = live_pool(&mut env, 0);
    let user = env.new_funded(2 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &user, 100_000_000_000);
    let r = swap(&mut env, &user, &p, 50_000_000_000, u64::MAX, false);
    expect_err!(r, DeepPoolError::SlippageExceeded);
}

#[test]
fn buy_zero_input_rejected() {
    let mut env = Env::new();
    let (p, _mint, _authority) = live_pool(&mut env, 0);
    let user = env.new_funded(2 * LAMPORTS_PER_SOL);
    let r = swap(&mut env, &user, &p, 0, 0, true);
    expect_err!(r, DeepPoolError::ZeroInput);
}

#[test]
fn report_swap_cu() {
    // Diagnostic: prints CU for buy and sell on a hot (already-existing ATA)
    // and cold (first-time ATA via idempotent create) path. Run with:
    //   cargo test --test litesvm swap::report_swap_cu -- --nocapture
    use crate::harness::{build_create_ata_idempotent_ix, derive_ata, spl_ata_program_id};
    use deep_pool::accounts::Swap as SwapAccounts;
    use deep_pool::constants::TOKEN_2022_PROGRAM_ID;
    use anchor_lang::{InstructionData, ToAccountMetas};
    use solana_sdk::{instruction::Instruction, system_program};

    let mut env = Env::new();
    let (p, mint, authority) = live_pool(&mut env, 0);
    let user = env.new_funded(5 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &user, 100_000_000_000);

    // ---------- Cold buy (create ATA + swap, single tx) ----------
    let user_token = derive_ata(&user.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let create_ata = build_create_ata_idempotent_ix(
        &user.pubkey(),
        &user.pubkey(),
        &p.mint,
        &TOKEN_2022_PROGRAM_ID,
    );
    let _ = spl_ata_program_id(); // suppress unused-warning if removed later
    let swap_ix = Instruction {
        program_id: deep_pool::ID,
        accounts: SwapAccounts {
            user: user.pubkey(),
            sol_source: user.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.token_vault,
            user_token_account: user_token,
            token_program: TOKEN_2022_PROGRAM_ID,
            system_program: system_program::ID,
            event_authority: crate::harness::derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::Swap {
            args: deep_pool::instructions::swap::SwapArgs {
                amount_in: 100_000_000,
                minimum_out: 1,
                buy: true,
            },
        }
        .data(),
    };
    let cu_cold = env
        .send_with_cu(&[create_ata, swap_ix.clone()], &[&user])
        .expect("cold swap");
    eprintln!("CU (cold: createATA + swap, one tx) = {}", cu_cold);

    // ---------- Hot buy (ATA already exists, swap only) ----------
    let cu_hot_buy = env.send_with_cu(&[swap_ix.clone()], &[&user]).expect("hot buy");
    eprintln!("CU (hot buy, swap-only) = {}", cu_hot_buy);

    // ---------- Hot sell (ATA already exists, swap only) ----------
    let sell_ix = Instruction {
        program_id: deep_pool::ID,
        accounts: SwapAccounts {
            user: user.pubkey(),
            sol_source: user.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.token_vault,
            user_token_account: user_token,
            token_program: TOKEN_2022_PROGRAM_ID,
            system_program: system_program::ID,
            event_authority: crate::harness::derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::Swap {
            args: deep_pool::instructions::swap::SwapArgs {
                amount_in: 10_000_000,
                minimum_out: 1,
                buy: false,
            },
        }
        .data(),
    };
    let cu_hot_sell = env.send_with_cu(&[sell_ix], &[&user]).expect("hot sell");
    eprintln!("CU (hot sell, swap-only) = {}", cu_hot_sell);
}

#[test]
fn k_invariant_grows_on_buy() {
    let mut env = Env::new();
    let (p, _mint, _authority) = live_pool(&mut env, 0);

    let sol_before = env.balance(&p.pool);
    let tokens_before = get_token_amount(&env, &p.token_vault);
    let k_before = (sol_before as u128) * (tokens_before as u128);

    let user = env.new_funded(5 * LAMPORTS_PER_SOL);
    swap(&mut env, &user, &p, 1 * LAMPORTS_PER_SOL, 1, true).expect("buy");

    let sol_after = env.balance(&p.pool);
    let tokens_after = get_token_amount(&env, &p.token_vault);
    let k_after = (sol_after as u128) * (tokens_after as u128);

    // Fees accrue to the pool, so k must strictly grow on every fee-bearing swap.
    assert!(k_after > k_before, "k must grow on fee-bearing buy");
}
