// The writer task.
//
// Receives BlockBatches from the gRPC subscriber, writes each atomically
// (events + reserves + checkpoint) in a single Postgres transaction, then
// broadcasts inserted rows to WS subscribers post-COMMIT.
//
// One BlockBatch = one transaction. The batch arrives pre-formed (all events
// from one block), so this layer only buffers on the channel.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::contracts::{
    BlockBatch, BroadcastFrame, Broadcaster, DecodedEvent, DeepPoolEvent, LiquidityRow,
    NewLiquidityRow, NewPoolRow, NewReservesRow, NewSwapRow, PoolRow, ReservesRow, SwapRow,
};
use crate::domain::{liquidity, pool, reserves, swap, PoolFilter};
use crate::stream::translate;

pub async fn run_writer(
    db: PgPool,
    broadcaster: Broadcaster,
    mut rx: mpsc::Receiver<BlockBatch>,
) -> anyhow::Result<()> {
    info!("writer task started");

    let mut pool_cache = load_pool_cache(&db).await?;
    let mut lp_supply_cache = load_lp_supply_cache(&db).await?;
    info!(
        pools = pool_cache.len(),
        lp_supply_entries = lp_supply_cache.len(),
        "caches loaded"
    );

    while let Some(batch) = rx.recv().await {
        let slot = batch.slot;
        match write_block(&db, &mut pool_cache, &mut lp_supply_cache, &batch).await {
            Ok(written) => {
                let n = written.pools.len()
                    + written.reserves.len()
                    + written.swaps.len()
                    + written.liquidity.len();
                if n > 0 {
                    info!(slot, inserted = n, "wrote block");
                }

                // POST-COMMIT broadcast. If commit failed we never get here,
                // so subscribers can't observe a row that a rollback erased.
                for r in written.pools {
                    broadcaster.publish(BroadcastFrame::Pool(Arc::new(r)));
                }
                for r in written.reserves {
                    broadcaster.publish(BroadcastFrame::Reserves(Arc::new(r)));
                }
                for r in written.swaps {
                    broadcaster.publish(BroadcastFrame::Swap(Arc::new(r)));
                }
                for r in written.liquidity {
                    broadcaster.publish(BroadcastFrame::Liquidity(Arc::new(r)));
                }
            }
            Err(e) => {
                // Don't advance the checkpoint; Laserstream replay on the
                // next restart will re-process from last_processed_slot - K.
                error!(slot, error = %e, "block write failed; checkpoint not advanced");
            }
        }
    }
    info!("writer task exiting (channel closed)");
    Ok(())
}

struct WrittenBlock {
    pools: Vec<PoolRow>,
    reserves: Vec<ReservesRow>,
    swaps: Vec<SwapRow>,
    liquidity: Vec<LiquidityRow>,
}

async fn write_block(
    db: &PgPool,
    pool_cache: &mut HashMap<String, i32>,
    lp_supply_cache: &mut HashMap<i32, i64>,
    batch: &BlockBatch,
) -> anyhow::Result<WrittenBlock> {
    let mut tx = db.begin().await?;

    // Deterministic processing order. Within a tx, inner_ix_idx is the natural
    // order. Across txs in a block, signature is a stable but arbitrary
    // tiebreaker — two events from different txs touching the same pool can
    // land in either order. Acceptable for v1; if it matters, add tx_index to
    // DecodedEvent at the decoder.
    let mut events: Vec<&DecodedEvent> = batch.events.iter().collect();
    events.sort_by(|a, b| {
        a.signature
            .cmp(&b.signature)
            .then(a.inner_ix_idx.cmp(&b.inner_ix_idx))
    });

    // Pass 1: insert pools, populate cache with new ids.
    let new_pools: Vec<NewPoolRow> = events
        .iter()
        .filter_map(|de| match &de.event {
            DeepPoolEvent::PoolCreated(p) => Some(translate::new_pool(p, de)),
            _ => None,
        })
        .collect();
    let inserted_pools = pool::set(&mut tx, &new_pools).await?;
    for r in &inserted_pools {
        pool_cache.insert(r.pubkey.clone(), r.pool_id);
    }

    // Backfill cache for pools we don't yet know about. Hits two cases:
    // (1) PoolCreated already in DB but not in cache (replay or restart with
    // newer pools than load_pool_cache fetched), and (2) swap / liquidity
    // events for pools whose PoolCreated was never seen by this writer.
    let mut needed: Vec<String> = events
        .iter()
        .map(|de| translate::pool_pubkey(&de.event))
        .filter(|pk| !pool_cache.contains_key(pk))
        .collect();
    needed.sort();
    needed.dedup();
    if !needed.is_empty() {
        let rows = pool::list(
            &mut tx,
            PoolFilter {
                pubkeys: Some(needed),
                ..Default::default()
            },
        )
        .await?;
        for r in rows {
            pool_cache.insert(r.pubkey.clone(), r.pool_id);
        }
    }

    // Pass 2: build per-table batches. Reserves rows mirror lp_supply forward
    // for swap events (which don't carry lp_supply on the event payload).
    let mut new_swaps = Vec::<NewSwapRow>::new();
    let mut new_liquidity = Vec::<NewLiquidityRow>::new();
    let mut new_reserves = Vec::<NewReservesRow>::new();

    for de in &events {
        let pubkey = translate::pool_pubkey(&de.event);
        let pool_id = match pool_cache.get(&pubkey) {
            Some(id) => *id,
            None => {
                warn!(slot = de.slot, sig = %de.signature, %pubkey, "unknown pool; skipping event");
                continue;
            }
        };

        match &de.event {
            DeepPoolEvent::PoolCreated(p) => {
                lp_supply_cache.insert(pool_id, p.lp_supply_after as i64);
                new_reserves.push(translate::new_reserves(
                    pool_id,
                    p.sol_reserve_after as i64,
                    p.token_reserve_after as i64,
                    p.lp_supply_after as i64,
                    de,
                ));
            }
            DeepPoolEvent::SwapExecuted(s) => {
                let lp_supply = lp_supply_cache.get(&pool_id).copied().unwrap_or(0);
                new_swaps.push(translate::new_swap(s, pool_id, de));
                new_reserves.push(translate::new_reserves(
                    pool_id,
                    s.sol_reserve_after as i64,
                    s.token_reserve_after as i64,
                    lp_supply,
                    de,
                ));
            }
            DeepPoolEvent::LiquidityAdded(la) => {
                lp_supply_cache.insert(pool_id, la.lp_supply_after as i64);
                new_liquidity.push(translate::new_liquidity_add(la, pool_id, de));
                new_reserves.push(translate::new_reserves(
                    pool_id,
                    la.sol_reserve_after as i64,
                    la.token_reserve_after as i64,
                    la.lp_supply_after as i64,
                    de,
                ));
            }
            DeepPoolEvent::LiquidityRemoved(lr) => {
                lp_supply_cache.insert(pool_id, lr.lp_supply_after as i64);
                new_liquidity.push(translate::new_liquidity_remove(lr, pool_id, de));
                new_reserves.push(translate::new_reserves(
                    pool_id,
                    lr.sol_reserve_after as i64,
                    lr.token_reserve_after as i64,
                    lr.lp_supply_after as i64,
                    de,
                ));
            }
        }
    }

    let inserted_reserves = reserves::set(&mut tx, &new_reserves).await?;
    let inserted_swaps = swap::set(&mut tx, &new_swaps).await?;
    let inserted_liquidity = liquidity::set(&mut tx, &new_liquidity).await?;

    // Checkpoint. INSERT...ON CONFLICT handles cold-start (no row yet) and
    // every subsequent block uniformly.
    sqlx::query(
        "INSERT INTO indexer_state (id, last_processed_slot) VALUES (1, $1)
         ON CONFLICT (id) DO UPDATE SET last_processed_slot = EXCLUDED.last_processed_slot",
    )
    .bind(batch.slot as i64)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(WrittenBlock {
        pools: inserted_pools,
        reserves: inserted_reserves,
        swaps: inserted_swaps,
        liquidity: inserted_liquidity,
    })
}

async fn load_pool_cache(db: &PgPool) -> sqlx::Result<HashMap<String, i32>> {
    let rows: Vec<(String, i32)> = sqlx::query_as("SELECT pubkey, pool_id FROM pools")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn load_lp_supply_cache(db: &PgPool) -> sqlx::Result<HashMap<i32, i64>> {
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT DISTINCT ON (pool_id) pool_id, lp_supply
         FROM reserves
         ORDER BY pool_id, last_slot DESC, reserve_id DESC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}
