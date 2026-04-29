// Translation layer: DecodedEvent → DB insert shapes.
//
// Pure mapping, no IO. Lives between the decoder (event payloads) and the
// writer (per-table batches). Pubkey [u8; 32] arrays become base58 strings.
// u64 → i64 cast is safe at realistic SOL/token magnitudes (i64 holds up to
// 9.2e18; max Solana lamports ≈ 4.3e15).

use chrono::{DateTime, Utc};

use crate::contracts::{
    DecodedEvent, DeepPoolEvent, LiquidityAdded, LiquidityRemoved, NewLiquidityRow, NewPoolRow,
    NewReservesRow, NewSwapRow, PoolCreated, SwapExecuted,
};

pub fn pool_pubkey(event: &DeepPoolEvent) -> String {
    b58(pool_bytes(event))
}

fn pool_bytes(event: &DeepPoolEvent) -> &[u8; 32] {
    match event {
        DeepPoolEvent::PoolCreated(p) => &p.pool,
        DeepPoolEvent::SwapExecuted(s) => &s.pool,
        DeepPoolEvent::LiquidityAdded(la) => &la.pool,
        DeepPoolEvent::LiquidityRemoved(lr) => &lr.pool,
    }
}

fn b58(bytes: &[u8; 32]) -> String {
    bs58::encode(bytes).into_string()
}

fn ts(de: &DecodedEvent) -> DateTime<Utc> {
    de.block_time.unwrap_or_else(Utc::now)
}

pub fn new_pool(p: &PoolCreated, de: &DecodedEvent) -> NewPoolRow {
    NewPoolRow {
        pubkey: b58(&p.pool),
        config: b58(&p.config),
        token_mint: b58(&p.token_mint),
        lp_mint: b58(&p.lp_mint),
        creator: b58(&p.creator),
        sol_initial: p.sol_in_net as i64,
        tokens_initial: p.tokens_in_net as i64,
        lp_supply_initial: p.lp_supply_after as i64,
        slot: de.slot,
        signature: de.signature.clone(),
        created_at: ts(de),
    }
}

pub fn new_swap(s: &SwapExecuted, pool_id: i32, de: &DecodedEvent) -> NewSwapRow {
    NewSwapRow {
        pool_id,
        user_pk: b58(&s.user),
        sol_source: b58(&s.sol_source),
        is_buy: s.buy,
        amount_in_gross: s.amount_in_gross as i64,
        amount_in_net: s.amount_in_net as i64,
        amount_out_gross: s.amount_out_gross as i64,
        amount_out_net: s.amount_out_net as i64,
        fee: s.fee as i64,
        sol_reserve_after: s.sol_reserve_after as i64,
        token_reserve_after: s.token_reserve_after as i64,
        slot: de.slot,
        signature: de.signature.clone(),
        inner_ix_idx: de.inner_ix_idx,
        created_at: ts(de),
    }
}

pub fn new_liquidity_add(la: &LiquidityAdded, pool_id: i32, de: &DecodedEvent) -> NewLiquidityRow {
    NewLiquidityRow {
        pool_id,
        provider: b58(&la.provider),
        is_add: true,
        sol_amount_gross: la.sol_in_gross as i64,
        sol_amount_net: la.sol_in_net as i64,
        tokens_amount_gross: la.tokens_in_gross as i64,
        tokens_amount_net: la.tokens_in_net as i64,
        lp_user_amount: la.lp_to_provider as i64,
        lp_locked: la.lp_locked as i64,
        lp_supply_after: la.lp_supply_after as i64,
        slot: de.slot,
        signature: de.signature.clone(),
        inner_ix_idx: de.inner_ix_idx,
        created_at: ts(de),
    }
}

pub fn new_liquidity_remove(
    lr: &LiquidityRemoved,
    pool_id: i32,
    de: &DecodedEvent,
) -> NewLiquidityRow {
    NewLiquidityRow {
        pool_id,
        provider: b58(&lr.provider),
        is_add: false,
        sol_amount_gross: lr.sol_out_gross as i64,
        sol_amount_net: lr.sol_out_net as i64,
        tokens_amount_gross: lr.tokens_out_gross as i64,
        tokens_amount_net: lr.tokens_out_net as i64,
        lp_user_amount: lr.lp_burned as i64,
        lp_locked: 0,
        lp_supply_after: lr.lp_supply_after as i64,
        slot: de.slot,
        signature: de.signature.clone(),
        inner_ix_idx: de.inner_ix_idx,
        created_at: ts(de),
    }
}

pub fn new_reserves(
    pool_id: i32,
    sol_reserve: i64,
    token_reserve: i64,
    lp_supply: i64,
    de: &DecodedEvent,
) -> NewReservesRow {
    NewReservesRow {
        pool_id,
        sol_reserve,
        token_reserve,
        lp_supply,
        last_slot: de.slot,
        signature: de.signature.clone(),
        inner_ix_idx: de.inner_ix_idx,
        created_at: ts(de),
    }
}
