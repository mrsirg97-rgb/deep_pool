// Yellowstone gRPC (= Helius Laserstream) block subscription.
//
// Subscribes to filtered blocks, decodes deep_pool events from each tx's
// inner instructions, yields BlockBatches to a channel.
//
// On reconnect, resumes from `last_processed_slot - reorg_buffer` so nothing
// is lost. Downstream idempotent UPSERTs de-dupe replays.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::DateTime;
use futures::{sink::SinkExt, stream::StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestFilterBlocks,
    SubscribeRequestPing,
};

use crate::contracts::{BlockBatch, DecodedEvent};
use crate::stream::decoder::{try_decode_event, Discriminators};

// Loops forever, reconnecting on error. `resume_slot` is
// `last_processed_slot - reorg_buffer` on first call. On retention-window
// rejection (slot too old for Laserstream), falls back to current tip.
pub async fn run_subscriber(
    laserstream_url: String,
    laserstream_token: String,
    program_id: String,
    resume_slot: u64,
    tx: mpsc::Sender<BlockBatch>,
) -> Result<()> {
    let discs = Discriminators::compute();
    info!(%program_id, resume_slot, "connecting to Laserstream");

    let mut current_resume = resume_slot;

    loop {
        match subscribe_once(
            &laserstream_url,
            &laserstream_token,
            &program_id,
            current_resume,
            &discs,
            &tx,
        )
        .await
        {
            Ok(()) => {
                warn!("gRPC stream ended cleanly; reconnecting");
            }
            Err(e) => {
                let chain = format!("{e:#}");
                if chain.contains("older than the oldest available slot") {
                    warn!(
                        previous_resume = current_resume,
                        "checkpoint older than Laserstream retention; falling back to current tip"
                    );
                    current_resume = 0;
                } else {
                    error!(error = chain, "gRPC stream errored; reconnecting in 2s");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
}

async fn subscribe_once(
    url: &str,
    token: &str,
    program_id: &str,
    from_slot: u64,
    discs: &Discriminators,
    tx: &mpsc::Sender<BlockBatch>,
) -> Result<()> {
    let mut client = GeyserGrpcClient::build_from_shared(url.to_string())?
        .x_token(Some(token.to_string()))?
        // The URL is https, but the builder doesn't auto-enable TLS. We must
        // pass an explicit TLS config or the HTTP/2 client tries plaintext and
        // the server closes the connection. `with_native_roots()` uses the
        // system CA bundle.
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .connect()
        .await
        .context("connect to Laserstream")?;

    let (mut subscribe_tx, mut stream) = client.subscribe().await?;

    // Server-side `account_include` narrows the delivered tx set to those
    // touching the deep_pool program. Empty blocks still arrive (with zero
    // filtered txs), which keeps the checkpoint advancing every slot.
    let mut blocks = HashMap::new();
    blocks.insert(
        "deep_pool_blocks".to_string(),
        SubscribeRequestFilterBlocks {
            account_include: vec![program_id.to_string()],
            include_transactions: Some(true),
            include_accounts: Some(false),
            include_entries: Some(false),
        },
    );

    // On fresh boot (checkpoint=0) or after a long downtime, skip the resume
    // and start from current tip. The first block we process seeds
    // `last_processed_slot` for future restarts.
    let effective_from_slot = if from_slot > 0 { Some(from_slot) } else { None };

    let request = SubscribeRequest {
        blocks,
        commitment: Some(CommitmentLevel::Confirmed as i32),
        from_slot: effective_from_slot,
        ..Default::default()
    };

    subscribe_tx
        .send(request)
        .await
        .context("send subscribe request")?;

    info!(from_slot = ?effective_from_slot, "subscribed to blocks");

    let program_bytes = bs58::decode(program_id)
        .into_vec()
        .context("invalid program_id")?;

    while let Some(update) = stream.next().await {
        let update = update.context("recv stream update")?;
        match update.update_oneof {
            Some(UpdateOneof::Block(block)) => {
                let slot = block.slot;
                let block_time = block
                    .block_time
                    .and_then(|bt| DateTime::from_timestamp(bt.timestamp, 0));

                let tx_count = block.transactions.len();
                if tx_count > 0 {
                    debug!(slot, tx_count, "block received with txs");
                }

                let mut events = Vec::new();
                for tx_update in &block.transactions {
                    let Some(meta) = tx_update.meta.as_ref() else {
                        continue;
                    };
                    let Some(tx_info) = tx_update.transaction.as_ref() else {
                        continue;
                    };

                    let signature = tx_info
                        .signatures
                        .first()
                        .map(|s| bs58::encode(s).into_string())
                        .unwrap_or_default();

                    let account_keys = collect_account_keys(tx_info, meta);
                    let Some(program_idx) = account_keys
                        .iter()
                        .position(|k| k.as_slice() == program_bytes.as_slice())
                    else {
                        continue;
                    };

                    // The program may appear in account_keys without being
                    // invoked — e.g., deploy/upgrade txs reference the
                    // program as the upgrade target. Skip those; they don't
                    // carry events and shouldn't warn on missing self-CPIs.
                    let invoked_as_outer = tx_info
                        .message
                        .as_ref()
                        .map(|msg| {
                            msg.instructions
                                .iter()
                                .any(|ix| ix.program_id_index as usize == program_idx)
                        })
                        .unwrap_or(false);
                    let invoked_as_inner = meta
                        .inner_instructions
                        .iter()
                        .flat_map(|g| g.instructions.iter())
                        .any(|ix| ix.program_id_index as usize == program_idx);
                    if !invoked_as_outer && !invoked_as_inner {
                        continue;
                    }

                    let mut flat_idx: i32 = 0;
                    for inner in &meta.inner_instructions {
                        for ix in &inner.instructions {
                            if ix.program_id_index as usize == program_idx {
                                match try_decode_event(&ix.data, discs) {
                                    Ok(event) => {
                                        events.push(DecodedEvent {
                                            signature: signature.clone(),
                                            inner_ix_idx: flat_idx,
                                            slot: slot as i64,
                                            block_time,
                                            event,
                                        });
                                    }
                                    Err(e) => {
                                        let prefix_hex: String = ix
                                            .data
                                            .iter()
                                            .take(16)
                                            .map(|b| format!("{:02x}", b))
                                            .collect::<Vec<_>>()
                                            .join(" ");
                                        warn!(
                                            slot,
                                            sig = %signature,
                                            error = %e,
                                            data_len = ix.data.len(),
                                            first_16_bytes = %prefix_hex,
                                            "self-CPI did not decode as a deep_pool event"
                                        );
                                    }
                                }
                            }
                            flat_idx += 1;
                        }
                    }
                }

                let batch = BlockBatch { slot, events };
                if tx.send(batch).await.is_err() {
                    warn!("writer channel closed; exiting subscribe loop");
                    return Ok(());
                }
            }
            Some(UpdateOneof::Ping(_)) => {
                // Reply to ping to keep the connection alive.
                let _ = subscribe_tx
                    .send(SubscribeRequest {
                        ping: Some(SubscribeRequestPing { id: 1 }),
                        ..Default::default()
                    })
                    .await;
            }
            Some(UpdateOneof::Pong(_)) => {}
            Some(_) => {}
            None => {}
        }
    }

    Ok(())
}

// Join static + loaded-writable + loaded-readonly account keys in the
// canonical order Solana uses for `program_id_index` resolution.
fn collect_account_keys(
    tx: &yellowstone_grpc_proto::prelude::Transaction,
    meta: &yellowstone_grpc_proto::prelude::TransactionStatusMeta,
) -> Vec<Vec<u8>> {
    let mut keys: Vec<Vec<u8>> = Vec::new();
    if let Some(msg) = tx.message.as_ref() {
        keys.extend(msg.account_keys.iter().cloned());
    }
    keys.extend(meta.loaded_writable_addresses.iter().cloned());
    keys.extend(meta.loaded_readonly_addresses.iter().cloned());
    keys
}
