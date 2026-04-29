// HTTP + WS API.
//
// Endpoints:
//   GET  /healthz                  liveness probe
//   GET  /api/pools                list pools (filters: token_mint, creator)
//   GET  /api/pools/:pubkey        pool detail with current reserves
//   GET  /api/swaps                swap history (filters: pool_id, user, token_mint, since/before)
//   GET  /api/liquidity            liquidity history (filters: pool_id, provider, token_mint, is_add, since/before)
//   WS   /events                   post-COMMIT broadcast stream
//
// All HTTP handlers run inside a per-request REPEATABLE-READ Postgres
// transaction. Errors auto-rollback via sqlx's Transaction Drop impl;
// success commits explicitly.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, warn};

use crate::contracts::{AppState, LiquidityRow, PoolRow, ReservesRow, SwapRow};
use crate::domain::{LiquidityFilter, PoolFilter, SwapFilter};
use crate::services::RequestCtx;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/pools", get(list_pools))
        .route("/api/pools/:pubkey", get(get_pool))
        .route("/api/swaps", get(list_swaps))
        .route("/api/liquidity", get(list_liquidity))
        .route("/events", get(ws_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

// ---------- /api/pools ----------

#[derive(Debug, Deserialize)]
struct PoolsQuery {
    token_mint: Option<String>,
    creator: Option<String>,
    limit: Option<i64>,
}

async fn list_pools(
    State(state): State<AppState>,
    Query(q): Query<PoolsQuery>,
) -> Result<Json<Vec<PoolRow>>, ApiError> {
    let mut ctx = RequestCtx::begin(&state.pool).await?;
    let pools = if let Some(mint) = q.token_mint {
        ctx.pools().for_token_mints(vec![mint]).await?
    } else if let Some(creator) = q.creator {
        ctx.pools().for_creators(vec![creator]).await?
    } else {
        ctx.pools()
            .list(PoolFilter {
                limit: q.limit,
                ..Default::default()
            })
            .await?
    };
    ctx.commit().await?;
    Ok(Json(arc_owned(pools)))
}

// ---------- /api/pools/:pubkey ----------

#[derive(Debug, Serialize)]
struct PoolDetail {
    pool: PoolRow,
    reserves: Option<ReservesRow>,
}

async fn get_pool(
    State(state): State<AppState>,
    Path(pubkey): Path<String>,
) -> Result<Json<PoolDetail>, ApiError> {
    let mut ctx = RequestCtx::begin(&state.pool).await?;
    let pool = ctx
        .pools()
        .by_pubkey(&pubkey)
        .await?
        .ok_or(ApiError::NotFound)?;
    let reserves = ctx.reserves().latest_for_pool(pool.pool_id).await?;
    ctx.commit().await?;
    Ok(Json(PoolDetail {
        pool: (*pool).clone(),
        reserves: reserves.map(|r| (*r).clone()),
    }))
}

// ---------- /api/swaps ----------

#[derive(Debug, Deserialize)]
struct SwapsQuery {
    pool_id: Option<i32>,
    user: Option<String>,
    token_mint: Option<String>,
    since: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

async fn list_swaps(
    State(state): State<AppState>,
    Query(q): Query<SwapsQuery>,
) -> Result<Json<Vec<SwapRow>>, ApiError> {
    let mut ctx = RequestCtx::begin(&state.pool).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let swaps = if let Some(mint) = q.token_mint {
        ctx.swaps().for_token_mint(&mint, Some(limit)).await?
    } else {
        ctx.swaps()
            .list(SwapFilter {
                pool_ids: q.pool_id.map(|id| vec![id]),
                users: q.user.map(|u| vec![u]),
                since: q.since,
                before: q.before,
                limit: Some(limit),
                ..Default::default()
            })
            .await?
    };
    ctx.commit().await?;
    Ok(Json(arc_owned(swaps)))
}

// ---------- /api/liquidity ----------

#[derive(Debug, Deserialize)]
struct LiquidityQuery {
    pool_id: Option<i32>,
    provider: Option<String>,
    token_mint: Option<String>,
    is_add: Option<bool>,
    since: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

async fn list_liquidity(
    State(state): State<AppState>,
    Query(q): Query<LiquidityQuery>,
) -> Result<Json<Vec<LiquidityRow>>, ApiError> {
    let mut ctx = RequestCtx::begin(&state.pool).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let events = if let Some(mint) = q.token_mint {
        ctx.liquidity().for_token_mint(&mint, Some(limit)).await?
    } else {
        ctx.liquidity()
            .list(LiquidityFilter {
                pool_ids: q.pool_id.map(|id| vec![id]),
                providers: q.provider.map(|p| vec![p]),
                is_add: q.is_add,
                since: q.since,
                before: q.before,
                limit: Some(limit),
                ..Default::default()
            })
            .await?
    };
    ctx.commit().await?;
    Ok(Json(arc_owned(events)))
}

// ---------- WS /events ----------

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| ws_session(socket, state))
}

async fn ws_session(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.broadcaster.subscribe();
    debug!(
        subscribers = state.broadcaster.subscriber_count(),
        "ws client connected"
    );

    loop {
        tokio::select! {
            biased;

            // Detect client disconnect / close.
            msg = receiver.next() => {
                match msg {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    _ => {}
                }
            }

            // Forward broadcast frames as JSON text.
            frame = rx.recv() => {
                match frame {
                    Ok(f) => {
                        let json = serde_json::to_string(&f).unwrap_or_default();
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        // Slow client got dropped from the broadcast queue.
                        // Surface so we can tune capacity if it's frequent.
                        warn!(skipped = n, "ws subscriber lagged");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
    debug!("ws client disconnected");
}

// ---------- helpers ----------

fn arc_owned<T: Clone>(arcs: Vec<Arc<T>>) -> Vec<T> {
    arcs.into_iter().map(|a| (*a).clone()).collect()
}

#[derive(Debug)]
enum ApiError {
    Db(sqlx::Error),
    NotFound,
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Db(e) => {
                tracing::error!(error = %e, "db error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
        }
    }
}
