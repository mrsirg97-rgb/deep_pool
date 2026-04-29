use std::collections::HashMap;
use std::sync::Arc;

use sqlx::{PgPool, Postgres, Transaction};

use crate::contracts::{PoolRow, ReservesRow};

// Per-request memo cache. Lives for the lifetime of the request transaction;
// dropped on commit/rollback. Service methods check the cache before issuing
// SQL so the same lookup within a request hits the DB at most once.
#[derive(Default)]
pub struct Cache {
    pub pools_by_id: HashMap<i32, Arc<PoolRow>>,
    pub pools_by_pubkey: HashMap<String, Arc<PoolRow>>,
    pub reserves_by_pool_id: HashMap<i32, Arc<ReservesRow>>,
}

// Request-scoped state. Owns the per-request transaction (REPEATABLE READ
// snapshot) and the memo cache. Services consume `&mut RequestCtx` and call
// domain methods through `&mut self.tx`.
pub struct RequestCtx {
    pub tx: Transaction<'static, Postgres>,
    pub cache: Cache,
}

impl RequestCtx {
    // Open a transaction at REPEATABLE READ for snapshot consistency across
    // every query in the request. Postgres requires SET TRANSACTION ISOLATION
    // to be the first statement after BEGIN; sqlx's `pool.begin()` issues
    // BEGIN, so this is the next thing we run.
    pub async fn begin(pool: &PgPool) -> sqlx::Result<Self> {
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
        Ok(Self {
            tx,
            cache: Cache::default(),
        })
    }

    pub async fn commit(self) -> sqlx::Result<()> {
        self.tx.commit().await
    }

    pub async fn rollback(self) -> sqlx::Result<()> {
        self.tx.rollback().await
    }
}
