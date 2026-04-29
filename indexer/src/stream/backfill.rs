// Historical backfill via JSON-RPC.
//
// Walks `getSignaturesForAddress(programId, { before: <oldest> })` backward
// page by page, fetches each tx via `getTransaction`, decodes the four
// deep_pool events through the same try_decode_event path live ingest uses,
// and inserts via the domain layer. One Postgres transaction per page.
//
// Independent of the live subscriber — runs as a one-shot subcommand and
// exits. Never touches `indexer_state.last_processed_slot` (that's the live
// indexer's checkpoint, must not regress).
//
// Stop conditions:
//   - getSignaturesForAddress returns an empty page → reached start of program
//   - a page produces zero new inserts → already caught up to live state
//
// Ordering caveat: walks newest → oldest, so reserve_id (SERIAL) reflects
// page-internal on-chain order correctly but inverts across pages. The
// "latest reserves" query orders by (last_slot DESC, reserve_id DESC), so
// the dominant slot key compensates — only same-slot events split across
// pages would tiebreak in the wrong direction (vanishingly rare).

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::contracts::{
    DecodedEvent, DeepPoolEvent, NewLiquidityRow, NewPoolRow, NewReservesRow, NewSwapRow,
};
use crate::domain::{liquidity, pool, reserves, swap, PoolFilter};
use crate::stream::decoder::{try_decode_event, Discriminators};
use crate::stream::translate;

// Solana RPC max for getSignaturesForAddress is 1000.
const PAGE_LIMIT: usize = 1000;

// Throttle between getTransaction calls. Public api.devnet.solana.com caps
// unauthenticated traffic around ~10 RPS, so 100ms is the safe default. With
// a paid RPC (Helius) you can set BACKFILL_TX_THROTTLE_MS=10 or lower.
const DEFAULT_TX_THROTTLE_MS: u64 = 100;

// Per-request retry budget for transient HTTP errors (429, 5xx).
const MAX_HTTP_ATTEMPTS: usize = 4;

#[derive(Deserialize, Debug)]
struct SignatureEntry {
    signature: String,
    slot: u64,
    #[serde(rename = "blockTime")]
    block_time: Option<i64>,
}

pub async fn run(rpc_url: String, program_id: String, db: PgPool) -> Result<()> {
    let discs = Discriminators::compute();
    let throttle_ms = std::env::var("BACKFILL_TX_THROTTLE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TX_THROTTLE_MS);
    let throttle = std::time::Duration::from_millis(throttle_ms);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build reqwest client")?;

    info!(%program_id, %rpc_url, throttle_ms, "starting backfill");

    let mut pool_cache = load_pool_cache(&db).await?;
    let mut lp_supply_cache = load_lp_supply_cache(&db).await?;

    let mut before: Option<String> = None;
    let mut total_sigs = 0usize;
    let mut total_events = 0usize;
    let mut total_inserted = 0usize;
    let mut page_num = 0usize;

    loop {
        page_num += 1;
        let sigs = get_signatures(&client, &rpc_url, &program_id, before.as_deref())
            .await
            .with_context(|| format!("fetch signatures page {page_num}"))?;
        if sigs.is_empty() {
            info!(page_num, "empty page; reached start of program history");
            break;
        }

        let oldest_sig = sigs.last().unwrap().signature.clone();
        let newest_slot = sigs.first().unwrap().slot;
        let oldest_slot = sigs.last().unwrap().slot;
        info!(
            page_num,
            page_size = sigs.len(),
            slot_range = format!("{newest_slot} → {oldest_slot}"),
            "fetched page"
        );
        total_sigs += sigs.len();

        // Decode every tx in the page (oldest-first within page so per-page
        // reserve_ids reflect on-chain order). Throttle between RPC calls.
        let mut decoded: Vec<DecodedEvent> = Vec::new();
        for entry in sigs.iter().rev() {
            match decode_tx(&client, &rpc_url, entry, &discs, &program_id).await {
                Ok(events) => decoded.extend(events),
                Err(e) => {
                    warn!(sig = %entry.signature, error = %e, "decode failed; skipping");
                }
            }
            if !throttle.is_zero() {
                tokio::time::sleep(throttle).await;
            }
        }
        total_events += decoded.len();

        let new_in_page = apply_page(&db, &mut pool_cache, &mut lp_supply_cache, decoded).await?;
        total_inserted += new_in_page;
        info!(page_num, new_in_page, "page processed");

        if new_in_page == 0 {
            // Page produced no new inserts → caught up to live state.
            // Idempotency means we could keep walking; stopping early just
            // saves RPC calls.
            info!("page produced no new inserts; backfill caught up");
            break;
        }

        before = Some(oldest_sig);
    }

    info!(
        total_pages = page_num,
        total_sigs,
        total_events,
        total_inserted,
        "backfill complete"
    );
    Ok(())
}

// Apply one page worth of decoded events in a single transaction. Returns
// the number of newly inserted rows across all four tables. Mirrors
// stream::writer::write_block but spans multiple slots and never touches
// indexer_state.
async fn apply_page(
    db: &PgPool,
    pool_cache: &mut HashMap<String, i32>,
    lp_supply_cache: &mut HashMap<i32, i64>,
    events: Vec<DecodedEvent>,
) -> Result<usize> {
    if events.is_empty() {
        return Ok(0);
    }

    // Sort ASC by (slot, signature, inner_ix_idx) so reserve_id assignment
    // within the page reflects on-chain order.
    let mut events = events;
    events.sort_by(|a, b| {
        a.slot
            .cmp(&b.slot)
            .then(a.signature.cmp(&b.signature))
            .then(a.inner_ix_idx.cmp(&b.inner_ix_idx))
    });

    let mut tx = db.begin().await?;

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

    // Backfill cache for any pool referenced but not yet known.
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

    tx.commit().await?;

    Ok(inserted_pools.len()
        + inserted_reserves.len()
        + inserted_swaps.len()
        + inserted_liquidity.len())
}

async fn get_signatures(
    client: &reqwest::Client,
    rpc_url: &str,
    program_id: &str,
    before: Option<&str>,
) -> Result<Vec<SignatureEntry>> {
    let mut config = serde_json::Map::new();
    config.insert("limit".to_string(), json!(PAGE_LIMIT));
    if let Some(b) = before {
        config.insert("before".to_string(), json!(b));
    }
    config.insert("commitment".to_string(), json!("confirmed"));

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSignaturesForAddress",
        "params": [program_id, Value::Object(config)],
    });

    let resp: Value = rpc_call(client, rpc_url, &body, "getSignaturesForAddress").await?;
    if let Some(err) = resp.get("error") {
        return Err(anyhow!("getSignaturesForAddress rpc error: {err}"));
    }
    let result = resp
        .get("result")
        .ok_or_else(|| anyhow!("getSignaturesForAddress: missing result"))?;
    serde_json::from_value(result.clone()).context("parse signatures result")
}

// Single-call wrapper with retry-on-transient. Surfaces real HTTP status +
// a body snippet on terminal failure so logs show what actually went wrong
// (e.g. 429 from public devnet, 401 from Helius with the wrong API key).
async fn rpc_call(
    client: &reqwest::Client,
    rpc_url: &str,
    body: &Value,
    method: &str,
) -> Result<Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_HTTP_ATTEMPTS {
        match client.post(rpc_url).json(body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return resp
                        .json::<Value>()
                        .await
                        .with_context(|| format!("{method} decode body"));
                }
                let snippet = resp
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>();
                let transient = status.as_u16() == 429 || status.is_server_error();
                let err = anyhow!("{method} HTTP {status}: {snippet}");
                if transient && attempt < MAX_HTTP_ATTEMPTS {
                    let backoff = std::time::Duration::from_millis(250 * attempt as u64);
                    warn!(method, attempt, ?backoff, %status, "transient RPC error; backing off");
                    tokio::time::sleep(backoff).await;
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }
            Err(e) => {
                let err = anyhow::Error::new(e).context(format!("post {method}"));
                if attempt < MAX_HTTP_ATTEMPTS {
                    warn!(method, attempt, "network error; retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64))
                        .await;
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("{method}: exhausted retry budget")))
}

async fn decode_tx(
    client: &reqwest::Client,
    rpc_url: &str,
    entry: &SignatureEntry,
    discs: &Discriminators,
    program_id: &str,
) -> Result<Vec<DecodedEvent>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            entry.signature,
            {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0,
            }
        ],
    });

    let resp: Value = rpc_call(client, rpc_url, &body, "getTransaction").await?;
    if let Some(err) = resp.get("error") {
        return Err(anyhow!("getTransaction rpc error: {err}"));
    }
    let result = match resp.get("result") {
        Some(r) if !r.is_null() => r,
        _ => return Ok(Vec::new()), // tx pruned or not yet indexed
    };

    let block_time = entry
        .block_time
        .and_then(|bt| DateTime::from_timestamp(bt, 0));

    let meta = result.get("meta").ok_or_else(|| anyhow!("tx missing meta"))?;
    let transaction = result
        .get("transaction")
        .ok_or_else(|| anyhow!("tx missing transaction"))?;
    let message = transaction
        .get("message")
        .ok_or_else(|| anyhow!("tx missing message"))?;

    // account_keys order: static keys, then loaded writable, then loaded
    // readonly — same canonical order Solana uses for `program_id_index`.
    // Mirror grpc::collect_account_keys so both paths derive the same idx.
    let mut keys: Vec<String> = message
        .get("accountKeys")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("missing accountKeys"))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if let Some(la) = meta.get("loadedAddresses") {
        if let Some(w) = la.get("writable").and_then(|v| v.as_array()) {
            keys.extend(w.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
        if let Some(r) = la.get("readonly").and_then(|v| v.as_array()) {
            keys.extend(r.iter().filter_map(|v| v.as_str().map(str::to_string)));
        }
    }

    let program_idx = match keys.iter().position(|k| k == program_id) {
        Some(i) => i,
        None => return Ok(Vec::new()),
    };

    let inner_groups = meta
        .get("innerInstructions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut events = Vec::new();
    let mut flat_idx: i32 = 0;
    for group in &inner_groups {
        let instructions = group
            .get("instructions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        for ix in &instructions {
            let pid_idx = ix
                .get("programIdIndex")
                .and_then(|v| v.as_u64())
                .unwrap_or(u64::MAX) as usize;
            if pid_idx == program_idx {
                let data_b58 = ix.get("data").and_then(|v| v.as_str()).unwrap_or("");
                if let Ok(data) = bs58::decode(data_b58).into_vec() {
                    if let Ok(event) = try_decode_event(&data, discs) {
                        events.push(DecodedEvent {
                            signature: entry.signature.clone(),
                            inner_ix_idx: flat_idx,
                            slot: entry.slot as i64,
                            block_time,
                            event,
                        });
                    }
                }
            }
            flat_idx += 1;
        }
    }

    Ok(events)
}

async fn load_pool_cache(db: &PgPool) -> Result<HashMap<String, i32>> {
    let rows: Vec<(String, i32)> = sqlx::query_as("SELECT pubkey, pool_id FROM pools")
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn load_lp_supply_cache(db: &PgPool) -> Result<HashMap<i32, i64>> {
    let rows: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT DISTINCT ON (pool_id) pool_id, lp_supply
         FROM reserves
         ORDER BY pool_id, last_slot DESC, reserve_id DESC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().collect())
}

