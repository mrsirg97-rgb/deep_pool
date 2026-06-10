//! [P-4] Event-content assertions — the event stream is the indexer's input
//! contract (the torch_market indexer ingests it byte-for-byte; consumers must
//! never see a different world than the state machine). Each test decodes the
//! emit_cpi! payload from the tx's inner instructions — the same decode path
//! the indexer uses — and compares every load-bearing field against
//! independently-read chain state. Run with a transfer-fee mint so the
//! gross/net distinction is actually exercised. See docs/events.md.

use solana_sdk::{native_token::LAMPORTS_PER_SOL, pubkey::Pubkey, signature::Keypair, signer::Signer};

use crate::harness::{
    add_liquidity, create_mint, create_pool, derive_ata, extract_event, get_token_amount,
    mint_to_user, remove_liquidity, swap, Env, PoolCtx,
};
use deep_pool::constants::TOKEN_2022_PROGRAM_ID;
use deep_pool::events::{LiquidityAdded, LiquidityRemoved, PoolCreated, SwapExecuted};
use deep_pool::state::Pool;

const FEE_BPS: u16 = 100; // 1% Token-2022 transfer fee — makes gross ≠ net

fn reserves_now(env: &Env, pool: &Pubkey, vault: &Pubkey) -> (u64, u64) {
    let rent = env.svm.minimum_balance_for_rent_exemption(Pool::LEN);
    let sol = env.svm.get_account(pool).unwrap().lamports - rent;
    (sol, get_token_amount(env, vault))
}

fn lp_supply(env: &Env, lp_mint: &Pubkey) -> u64 {
    // SPL Mint layout: supply at offset 36..44.
    use solana_sdk::account::ReadableAccount;
    let acct = env.svm.get_account(lp_mint).unwrap();
    u64::from_le_bytes(acct.data()[36..44].try_into().unwrap())
}

fn fee_pool(env: &mut Env) -> (PoolCtx, Keypair, Keypair) {
    let (mint, authority) = create_mint(env, FEE_BPS, 6);
    let creator = env.new_funded(30 * LAMPORTS_PER_SOL);
    mint_to_user(env, &mint, &authority, &creator, 10_000_000_000_000);
    let p = create_pool(env, &creator, &mint, 1_000_000_000_000, 10 * LAMPORTS_PER_SOL)
        .expect("create_pool");
    (p, creator, authority)
}

#[test]
fn pool_created_event_matches_chain() {
    let mut env = Env::new();
    let (p, _creator, _) = fee_pool(&mut env);

    let ev: PoolCreated =
        extract_event(env.last_meta.as_ref().unwrap()).expect("PoolCreated emitted");
    let (sol, tok) = reserves_now(&env, &p.pool, &p.token_vault);

    assert_eq!(ev.pool, p.pool);
    assert_eq!(ev.token_mint, p.mint);
    assert_eq!(ev.sol_in_gross, 10 * LAMPORTS_PER_SOL);
    assert_eq!(ev.sol_in_net, ev.sol_in_gross, "SOL has no transfer fee");
    assert_eq!(ev.tokens_in_gross, 1_000_000_000_000);
    assert!(ev.tokens_in_net < ev.tokens_in_gross, "1% fee shaves the deposit");
    assert_eq!(ev.tokens_in_net, tok, "net == vault balance for a fresh pool");
    assert_eq!(ev.sol_reserve_after, sol);
    assert_eq!(ev.token_reserve_after, tok);
    assert_eq!(ev.lp_supply_after, lp_supply(&env, &p.lp_mint));
    assert_eq!(ev.lp_to_creator + ev.lp_locked, ev.lp_supply_after);
}

#[test]
fn swap_buy_event_matches_chain() {
    let mut env = Env::new();
    let (p, _, _) = fee_pool(&mut env);
    let user = env.new_funded(10 * LAMPORTS_PER_SOL);
    let user_ata = derive_ata(&user.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);

    swap(&mut env, &user, &p, LAMPORTS_PER_SOL, 1, true).expect("buy");
    let ev: SwapExecuted =
        extract_event(env.last_meta.as_ref().unwrap()).expect("SwapExecuted emitted");
    let (sol, tok) = reserves_now(&env, &p.pool, &p.token_vault);

    assert!(ev.buy);
    assert_eq!(ev.amount_in_gross, LAMPORTS_PER_SOL);
    assert_eq!(ev.amount_in_net, ev.amount_in_gross, "SOL leg: gross == net");
    assert_eq!(ev.fee, LAMPORTS_PER_SOL * 25 / 10_000, "0.25% pool fee");
    assert!(
        ev.amount_out_net < ev.amount_out_gross,
        "recipient-side Token-2022 fee must show in net"
    );
    assert_eq!(
        ev.amount_out_net,
        get_token_amount(&env, &user_ata),
        "amount_out_net == what the user actually holds"
    );
    assert_eq!(ev.sol_reserve_after, sol, "state-stamped reserve (resync point)");
    assert_eq!(ev.token_reserve_after, tok);
}

#[test]
fn swap_sell_event_matches_chain() {
    let mut env = Env::new();
    let (p, _, authority) = fee_pool(&mut env);
    let user = env.new_funded(10 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &p.mint, &authority, &user, 100_000_000_000);

    let vault_before = get_token_amount(&env, &p.token_vault);
    swap(&mut env, &user, &p, 50_000_000_000, 1, false).expect("sell");
    let ev: SwapExecuted =
        extract_event(env.last_meta.as_ref().unwrap()).expect("SwapExecuted emitted");
    let (sol, tok) = reserves_now(&env, &p.pool, &p.token_vault);

    assert!(!ev.buy);
    assert_eq!(ev.amount_in_gross, 50_000_000_000);
    assert_eq!(
        ev.amount_in_net,
        tok - vault_before,
        "amount_in_net == vault delta (the AMM math input)"
    );
    assert!(ev.amount_in_net < ev.amount_in_gross, "sender-side fee visible");
    assert_eq!(ev.amount_out_net, ev.amount_out_gross, "SOL out: gross == net");
    assert_eq!(ev.sol_reserve_after, sol);
    assert_eq!(ev.token_reserve_after, tok);
}

#[test]
fn liquidity_events_match_chain() {
    let mut env = Env::new();
    let (p, _, authority) = fee_pool(&mut env);
    let provider = env.new_funded(20 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &p.mint, &authority, &provider, 1_000_000_000_000);
    let provider_token = derive_ata(&provider.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let provider_lp = derive_ata(&provider.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);

    // --- add ---
    add_liquidity(&mut env, &provider, &p, 100_000_000_000, 5 * LAMPORTS_PER_SOL, 1)
        .expect("add_liquidity");
    let ev: LiquidityAdded =
        extract_event(env.last_meta.as_ref().unwrap()).expect("LiquidityAdded emitted");
    let (sol, tok) = reserves_now(&env, &p.pool, &p.token_vault);
    assert_eq!(ev.tokens_in_gross, 100_000_000_000);
    assert!(ev.tokens_in_net < ev.tokens_in_gross, "fee on the deposit leg");
    assert_eq!(ev.sol_in_net, ev.sol_in_gross);
    assert_eq!(ev.lp_to_provider, get_token_amount(&env, &provider_lp));
    assert_eq!(ev.sol_reserve_after, sol);
    assert_eq!(ev.token_reserve_after, tok);
    assert_eq!(ev.lp_supply_after, lp_supply(&env, &p.lp_mint));

    // --- remove ---
    let lp_bal = get_token_amount(&env, &provider_lp);
    let held_before = get_token_amount(&env, &provider_token);
    remove_liquidity(&mut env, &provider, &p, lp_bal / 2, 0, 0).expect("remove_liquidity");
    let ev: LiquidityRemoved =
        extract_event(env.last_meta.as_ref().unwrap()).expect("LiquidityRemoved emitted");
    let (sol, tok) = reserves_now(&env, &p.pool, &p.token_vault);
    assert_eq!(ev.lp_burned, lp_bal / 2);
    assert_eq!(ev.sol_out_net, ev.sol_out_gross, "SOL out: gross == net");
    assert!(ev.tokens_out_net < ev.tokens_out_gross, "recipient-side fee visible");
    assert_eq!(
        ev.tokens_out_net,
        get_token_amount(&env, &provider_token) - held_before,
        "tokens_out_net == provider delta"
    );
    assert_eq!(ev.sol_reserve_after, sol);
    assert_eq!(ev.token_reserve_after, tok);
    assert_eq!(ev.lp_supply_after, lp_supply(&env, &p.lp_mint));
}
