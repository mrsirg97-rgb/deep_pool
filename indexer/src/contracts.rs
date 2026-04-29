use std::sync::Arc;

use borsh::BorshDeserialize;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::broadcast;

use crate::constants::BROADCAST_CAPACITY;

// ============================================================================
// On-chain event decode shapes
// ============================================================================
//
// Borsh field ORDER mirrors programs/deep_pool/src/events.rs exactly. Borsh is
// positional — reordering silently breaks decode of historical events.

#[derive(borsh::BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct PoolCreated {
    pub pool: [u8; 32],
    pub config: [u8; 32],
    pub token_mint: [u8; 32],
    pub lp_mint: [u8; 32],
    pub creator: [u8; 32],
    pub sol_in_gross: u64,
    pub sol_in_net: u64,
    pub tokens_in_gross: u64,
    pub tokens_in_net: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
    pub lp_to_creator: u64,
    pub lp_locked: u64,
}

#[derive(borsh::BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct LiquidityAdded {
    pub pool: [u8; 32],
    pub provider: [u8; 32],
    pub sol_in_gross: u64,
    pub sol_in_net: u64,
    pub tokens_in_gross: u64,
    pub tokens_in_net: u64,
    pub lp_to_provider: u64,
    pub lp_locked: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
}

#[derive(borsh::BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct LiquidityRemoved {
    pub pool: [u8; 32],
    pub provider: [u8; 32],
    pub lp_burned: u64,
    pub sol_out_gross: u64,
    pub sol_out_net: u64,
    pub tokens_out_gross: u64,
    pub tokens_out_net: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
}

#[derive(borsh::BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct SwapExecuted {
    pub pool: [u8; 32],
    pub user: [u8; 32],
    pub sol_source: [u8; 32],
    pub buy: bool,
    pub amount_in_gross: u64,
    pub amount_in_net: u64,
    pub amount_out_gross: u64,
    pub amount_out_net: u64,
    pub fee: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub total_swaps: u64,
}

#[derive(Debug, Clone)]
pub enum DeepPoolEvent {
    PoolCreated(PoolCreated),
    SwapExecuted(SwapExecuted),
    LiquidityAdded(LiquidityAdded),
    LiquidityRemoved(LiquidityRemoved),
}

// ============================================================================
// Decoded events with tx context
// ============================================================================

#[derive(Debug, Clone)]
pub struct DecodedEvent {
    pub signature: String,
    pub inner_ix_idx: i32,
    pub slot: i64,
    pub block_time: Option<DateTime<Utc>>,
    pub event: DeepPoolEvent,
}

// One block's worth of decoded events. The unit pushed from the gRPC
// subscriber to the writer; one BlockBatch = one Postgres transaction.
#[derive(Debug)]
pub struct BlockBatch {
    pub slot: u64,
    pub events: Vec<DecodedEvent>,
}

// ============================================================================
// DB row types
// ============================================================================
//
// Pubkey columns use base58 String mapping to Postgres TEXT. Conversion from
// raw [u8; 32] event payloads happens when building rows from DecodedEvents.

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct PoolRow {
    pub pool_id: i32,
    pub pubkey: String,
    pub config: String,
    pub token_mint: String,
    pub lp_mint: String,
    pub creator: String,
    pub sol_initial: i64,
    pub tokens_initial: i64,
    pub lp_supply_initial: i64,
    pub slot: i64,
    pub signature: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct ReservesRow {
    pub reserve_id: i32,
    pub pool_id: i32,
    pub sol_reserve: i64,
    pub token_reserve: i64,
    pub lp_supply: i64,
    pub last_slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct SwapRow {
    pub swap_id: i32,
    pub pool_id: i32,
    pub user_pk: String,
    pub sol_source: String,
    pub is_buy: bool,
    pub amount_in_gross: i64,
    pub amount_in_net: i64,
    pub amount_out_gross: i64,
    pub amount_out_net: i64,
    pub fee: i64,
    pub sol_reserve_after: i64,
    pub token_reserve_after: i64,
    pub total_swaps: i64,
    pub slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct LiquidityRow {
    pub liquidity_id: i32,
    pub pool_id: i32,
    pub provider: String,
    pub is_add: bool,
    pub sol_amount_gross: i64,
    pub sol_amount_net: i64,
    pub tokens_amount_gross: i64,
    pub tokens_amount_net: i64,
    pub lp_user_amount: i64,
    pub lp_locked: i64,
    pub lp_supply_after: i64,
    pub slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Insert shapes (rows without DB-assigned SERIAL ids)
// ============================================================================

#[derive(Debug, Clone)]
pub struct NewPoolRow {
    pub pubkey: String,
    pub config: String,
    pub token_mint: String,
    pub lp_mint: String,
    pub creator: String,
    pub sol_initial: i64,
    pub tokens_initial: i64,
    pub lp_supply_initial: i64,
    pub slot: i64,
    pub signature: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReservesRow {
    pub pool_id: i32,
    pub sol_reserve: i64,
    pub token_reserve: i64,
    pub lp_supply: i64,
    pub last_slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSwapRow {
    pub pool_id: i32,
    pub user_pk: String,
    pub sol_source: String,
    pub is_buy: bool,
    pub amount_in_gross: i64,
    pub amount_in_net: i64,
    pub amount_out_gross: i64,
    pub amount_out_net: i64,
    pub fee: i64,
    pub sol_reserve_after: i64,
    pub token_reserve_after: i64,
    pub total_swaps: i64,
    pub slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewLiquidityRow {
    pub pool_id: i32,
    pub provider: String,
    pub is_add: bool,
    pub sol_amount_gross: i64,
    pub sol_amount_net: i64,
    pub tokens_amount_gross: i64,
    pub tokens_amount_net: i64,
    pub lp_user_amount: i64,
    pub lp_locked: i64,
    pub lp_supply_after: i64,
    pub slot: i64,
    pub signature: String,
    pub inner_ix_idx: i32,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Broadcast (post-COMMIT WS frames)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BroadcastFrame {
    Pool(Arc<PoolRow>),
    Swap(Arc<SwapRow>),
    Liquidity(Arc<LiquidityRow>),
    Reserves(Arc<ReservesRow>),
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub broadcaster: Broadcaster,
}

#[derive(Clone, Debug)]
pub struct Broadcaster {
    sender: broadcast::Sender<BroadcastFrame>,
}

impl Broadcaster {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BroadcastFrame> {
        self.sender.subscribe()
    }

    pub fn publish(&self, frame: BroadcastFrame) {
        // send returns Err only when there are zero subscribers — that's fine.
        let _ = self.sender.send(frame);
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}
