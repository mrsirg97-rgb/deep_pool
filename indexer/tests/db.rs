// Integration tests for the domain layer — exercises the per-table CRUD
// against a real Postgres instance.
//
// Each test gets a fresh ephemeral DB via #[sqlx::test], with the deep_pool
// schema loaded from db/01-schema.sql. DATABASE_URL must point at a Postgres
// the test process can CREATEDB on (the dev compose superuser works). Run:
//
//   docker compose up -d postgres
//   DATABASE_URL=postgres://deep_pool:<superuser-pw>@127.0.0.1:5432/deep_pool \
//     cargo test --test db

use chrono::{DateTime, TimeZone, Utc};
use deep_pool_indexer::contracts::{
    NewLiquidityRow, NewPoolRow, NewReservesRow, NewSwapRow,
};
use deep_pool_indexer::domain::{liquidity, pool, reserves, swap, PoolFilter, SwapFilter};
use sqlx::PgPool;

const SCHEMA_SQL: &str = include_str!("../../db/01-schema.sql");

async fn setup(pool: &PgPool) {
    sqlx::raw_sql(SCHEMA_SQL)
        .execute(pool)
        .await
        .expect("apply schema");
}

fn ts() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap()
}

fn sample_pool(pubkey: &str, slot: i64) -> NewPoolRow {
    NewPoolRow {
        pubkey: pubkey.to_string(),
        config: format!("config-{pubkey}"),
        token_mint: format!("mint-{pubkey}"),
        lp_mint: format!("lp-{pubkey}"),
        creator: format!("creator-{pubkey}"),
        sol_initial: 1_000_000_000,
        tokens_initial: 10_000_000_000_000,
        lp_supply_initial: 99_999_999_000,
        slot,
        signature: format!("sig-pool-{pubkey}"),
        created_at: ts(),
    }
}

fn sample_reserves(pool_id: i32, slot: i64, sig: &str) -> NewReservesRow {
    NewReservesRow {
        pool_id,
        sol_reserve: 1_000_000_000,
        token_reserve: 10_000_000_000_000,
        lp_supply: 99_999_999_000,
        last_slot: slot,
        signature: sig.to_string(),
        inner_ix_idx: 0,
        created_at: ts(),
    }
}

fn sample_swap(pool_id: i32, sig: &str, idx: i32) -> NewSwapRow {
    NewSwapRow {
        pool_id,
        user_pk: "user-1".into(),
        sol_source: "user-1".into(),
        is_buy: true,
        amount_in_gross: 100_000_000,
        amount_in_net: 100_000_000,
        amount_out_gross: 900_000_000_000,
        amount_out_net: 900_000_000_000,
        fee: 250_000,
        sol_reserve_after: 1_100_000_000,
        token_reserve_after: 9_100_000_000_000,
        total_swaps: 1,
        slot: 100,
        signature: sig.to_string(),
        inner_ix_idx: idx,
        created_at: ts(),
    }
}

fn sample_liq(pool_id: i32, sig: &str, idx: i32, is_add: bool) -> NewLiquidityRow {
    NewLiquidityRow {
        pool_id,
        provider: "provider-1".into(),
        is_add,
        sol_amount_gross: 120_000_000,
        sol_amount_net: 120_000_000,
        tokens_amount_gross: 1_000_000_000_000,
        tokens_amount_net: 1_000_000_000_000,
        lp_user_amount: if is_add { 10_000_000_000 } else { 9_000_000_000 },
        lp_locked: if is_add { 800_000_000 } else { 0 },
        lp_supply_after: 110_000_000_000,
        slot: 100,
        signature: sig.to_string(),
        inner_ix_idx: idx,
        created_at: ts(),
    }
}

// ---------- pool ----------

#[sqlx::test]
async fn pool_set_returns_inserted(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let inserted = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].pubkey, "PK1");
    assert!(inserted[0].pool_id > 0, "SERIAL pool_id assigned");
}

#[sqlx::test]
async fn pool_set_idempotent_on_pubkey(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let row = sample_pool("PK1", 100);

    let first = pool::set(&mut tx, &[row.clone()]).await.unwrap();
    assert_eq!(first.len(), 1, "first insert succeeds");

    let second = pool::set(&mut tx, &[row]).await.unwrap();
    assert_eq!(
        second.len(),
        0,
        "duplicate pubkey returns empty Vec — no double-broadcast",
    );
    tx.commit().await.unwrap();
}

#[sqlx::test]
async fn pool_list_filters_by_token_mint(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    pool::set(&mut tx, &[sample_pool("PK1", 100), sample_pool("PK2", 101)])
        .await
        .unwrap();

    let only_mint1 = pool::list(
        &mut tx,
        PoolFilter {
            token_mints: Some(vec!["mint-PK1".into()]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(only_mint1.len(), 1);
    assert_eq!(only_mint1[0].pubkey, "PK1");
    tx.commit().await.unwrap();
}

// ---------- reserves ----------

#[sqlx::test]
async fn reserves_append_only_and_latest_picks_highest_slot(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    // Append three snapshots at increasing slots
    let inserted = reserves::set(
        &mut tx,
        &[
            sample_reserves(p.pool_id, 100, "sig-r1"),
            sample_reserves(p.pool_id, 101, "sig-r2"),
            sample_reserves(p.pool_id, 102, "sig-r3"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(inserted.len(), 3, "all three snapshots written");

    let latest = reserves::latest_for_pools(&mut tx, &[p.pool_id])
        .await
        .unwrap();
    assert_eq!(latest.len(), 1);
    assert_eq!(
        latest[0].last_slot, 102,
        "latest reserves picks highest last_slot",
    );
    assert_eq!(latest[0].signature, "sig-r3");
    tx.commit().await.unwrap();
}

#[sqlx::test]
async fn reserves_latest_tiebreaks_by_reserve_id(db: PgPool) {
    // Within a slot, reserve_id (SERIAL = insertion order) breaks ties.
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    reserves::set(
        &mut tx,
        &[
            sample_reserves(p.pool_id, 100, "sig-r1"),
            sample_reserves(p.pool_id, 100, "sig-r2"),
            sample_reserves(p.pool_id, 100, "sig-r3"),
        ],
    )
    .await
    .unwrap();

    let latest = reserves::latest_for_pools(&mut tx, &[p.pool_id])
        .await
        .unwrap();
    assert_eq!(
        latest[0].signature, "sig-r3",
        "highest reserve_id wins within a slot",
    );
    tx.commit().await.unwrap();
}

#[sqlx::test]
async fn reserves_idempotent_on_signature_inner_ix(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    let row = sample_reserves(p.pool_id, 100, "sig-r1");
    let first = reserves::set(&mut tx, &[row.clone()]).await.unwrap();
    assert_eq!(first.len(), 1);

    let second = reserves::set(&mut tx, &[row]).await.unwrap();
    assert_eq!(second.len(), 0, "replay no-ops");
    tx.commit().await.unwrap();
}

// ---------- swap ----------

#[sqlx::test]
async fn swap_set_idempotent_on_signature_inner_ix(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    let row = sample_swap(p.pool_id, "sig-s1", 0);
    let first = swap::set(&mut tx, &[row.clone()]).await.unwrap();
    assert_eq!(first.len(), 1);
    let second = swap::set(&mut tx, &[row]).await.unwrap();
    assert_eq!(second.len(), 0, "replay no-ops");
    tx.commit().await.unwrap();
}

#[sqlx::test]
async fn swap_list_filters_by_pool_id(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p1 = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();
    let p2 = pool::set(&mut tx, &[sample_pool("PK2", 100)]).await.unwrap()[0].clone();

    swap::set(
        &mut tx,
        &[
            sample_swap(p1.pool_id, "sig-s1", 0),
            sample_swap(p2.pool_id, "sig-s2", 0),
        ],
    )
    .await
    .unwrap();

    let only_p1 = swap::list(
        &mut tx,
        SwapFilter {
            pool_ids: Some(vec![p1.pool_id]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(only_p1.len(), 1);
    assert_eq!(only_p1[0].pool_id, p1.pool_id);
    tx.commit().await.unwrap();
}

// ---------- liquidity ----------

#[sqlx::test]
async fn liquidity_handles_both_directions(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    liquidity::set(
        &mut tx,
        &[
            sample_liq(p.pool_id, "sig-add", 0, true),
            sample_liq(p.pool_id, "sig-remove", 1, false),
        ],
    )
    .await
    .unwrap();

    let all = liquidity::list(&mut tx, Default::default()).await.unwrap();
    assert_eq!(all.len(), 2);

    let adds = liquidity::list(
        &mut tx,
        deep_pool_indexer::domain::LiquidityFilter {
            is_add: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(adds.len(), 1);
    assert!(adds[0].is_add);
    assert!(adds[0].lp_locked > 0, "adds carry lp_locked");

    let removes = liquidity::list(
        &mut tx,
        deep_pool_indexer::domain::LiquidityFilter {
            is_add: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(removes.len(), 1);
    assert!(!removes[0].is_add);
    assert_eq!(
        removes[0].lp_locked, 0,
        "removes have lp_locked=0 by convention",
    );
    tx.commit().await.unwrap();
}

#[sqlx::test]
async fn liquidity_idempotent_on_signature_inner_ix(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    let p = pool::set(&mut tx, &[sample_pool("PK1", 100)]).await.unwrap()[0].clone();

    let row = sample_liq(p.pool_id, "sig-add", 0, true);
    let first = liquidity::set(&mut tx, &[row.clone()]).await.unwrap();
    assert_eq!(first.len(), 1);
    let second = liquidity::set(&mut tx, &[row]).await.unwrap();
    assert_eq!(second.len(), 0, "replay no-ops");
    tx.commit().await.unwrap();
}

// ---------- empty inputs ----------

#[sqlx::test]
async fn empty_inputs_short_circuit(db: PgPool) {
    setup(&db).await;
    let mut tx = db.begin().await.unwrap();
    assert_eq!(pool::set(&mut tx, &[]).await.unwrap().len(), 0);
    assert_eq!(reserves::set(&mut tx, &[]).await.unwrap().len(), 0);
    assert_eq!(swap::set(&mut tx, &[]).await.unwrap().len(), 0);
    assert_eq!(liquidity::set(&mut tx, &[]).await.unwrap().len(), 0);
    tx.commit().await.unwrap();
}
