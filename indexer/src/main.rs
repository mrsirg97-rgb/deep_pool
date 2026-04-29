// Bin entry point. Module code lives in lib.rs so integration tests under
// tests/ can import it. This file just wires the runtime (tokio, tracing,
// subcommand dispatch) and delegates everything else.

use anyhow::Context;
use deep_pool_indexer::constants::BLOCK_CHANNEL_CAPACITY;
use deep_pool_indexer::{
    api, config, contracts, db, stream::backfill, stream::grpc, stream::writer,
};
use tokio::sync::mpsc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deep_pool_indexer=info,sqlx=warn,tower_http=info".into()),
        )
        .with_target(false)
        .init();

    let cfg = config::Config::from_env().context("load config")?;

    // Subcommands. Default and `run` start the live indexer; `backfill` runs
    // a one-shot historical pass and exits. Backfill needs only DATABASE_URL +
    // RPC_URL + DEEP_POOL_PROGRAM_ID — Laserstream env vars can be unset for
    // that path.
    match std::env::args().nth(1).as_deref() {
        Some("backfill") => return run_backfill(cfg).await,
        Some("run") | None => {}
        Some(other) => {
            anyhow::bail!("unknown subcommand: {other:?}. expected `run` or `backfill`");
        }
    }

    run_live(cfg).await
}

async fn run_backfill(cfg: config::Config) -> anyhow::Result<()> {
    info!("running historical backfill");
    let pool = db::connect(&cfg.database_url)
        .await
        .context("connect to Postgres")?;
    backfill::run(cfg.rpc_url, cfg.program_id, pool).await
}

async fn run_live(cfg: config::Config) -> anyhow::Result<()> {
    info!("starting deep_pool indexer");

    let pool = db::connect(&cfg.database_url)
        .await
        .context("connect to Postgres")?;

    let last = db::last_processed_slot(&pool)
        .await
        .context("read checkpoint")?;
    let resume = last.saturating_sub(cfg.reorg_buffer_slots);
    info!(
        last_checkpoint = last,
        resume_from = resume,
        "resuming subscription"
    );

    let broadcaster = contracts::Broadcaster::new();
    let (tx, rx) = mpsc::channel::<contracts::BlockBatch>(BLOCK_CHANNEL_CAPACITY);

    // Writer task: drains the channel, writes per-block, post-COMMIT broadcast.
    let writer_handle = {
        let pool = pool.clone();
        let bc = broadcaster.clone();
        tokio::spawn(async move {
            if let Err(e) = writer::run_writer(pool, bc, rx).await {
                tracing::error!(error = %e, "writer task exited with error");
            }
        })
    };

    // Subscriber task: Laserstream → decoder → channel. Reconnects internally.
    let subscriber_handle = {
        let url = cfg.laserstream_url.clone();
        let token = cfg.laserstream_token.clone();
        let program = cfg.program_id.clone();
        tokio::spawn(async move {
            if let Err(e) = grpc::run_subscriber(url, token, program, resume, tx).await {
                tracing::error!(error = %e, "subscriber task exited with error");
            }
        })
    };

    // HTTP + WS server.
    let state = contracts::AppState {
        pool: pool.clone(),
        broadcaster: broadcaster.clone(),
    };
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.api_bind)
        .await
        .with_context(|| format!("bind {}", cfg.api_bind))?;
    info!(bind = %cfg.api_bind, "api server listening");

    let server_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "api server exited");
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
        }
        _ = writer_handle => {}
        _ = subscriber_handle => {}
        _ = server_handle => {}
    }

    info!("shutting down");
    Ok(())
}
