// Indexer HTTP client.
//
// Reads through the deep_pool indexer's API (typically running locally —
// `docker compose --profile full up`). Used by the pool getters in
// `getters.ts` as the fast path; falls back to RPC silently on any failure.
//
// Two categories of methods here:
//   1. Internal helper (`getPoolDetailFromIndexer`) consumed by getters.ts
//      to populate the indexer-first path with RPC fallback.
//   2. Public indexer-only methods (`getSwapHistory`, `getLiquidityHistory`)
//      that have no RPC equivalent — chain RPCs can't answer "all swaps for
//      pool X since time T". These throw if the indexer is unreachable.
//
// All wire types (IndexerPoolRow, ReservesRow, SwapRow, etc.) live in
// `./types` alongside the public PoolState / SwapQuote contracts.

import { LAMPORTS_PER_SOL, PublicKey } from '@solana/web3.js'

import { getVaultPda } from './pda'
import type {
  IndexerPoolDetail,
  LiquidityHistoryQuery,
  LiquidityRow,
  PoolState,
  SwapHistoryQuery,
  SwapRow,
} from './types'

// ============================================================================
// Internal: HTTP fetch + decode
// ============================================================================

async function indexerFetch<T>(indexer: string, path: string): Promise<T> {
  const resp = await fetch(`${indexer}${path}`)
  if (!resp.ok) {
    throw new Error(`indexer ${path}: HTTP ${resp.status}`)
  }
  return resp.json() as Promise<T>
}

function detailToPoolState(detail: IndexerPoolDetail): PoolState {
  const poolPubkey = new PublicKey(detail.pool.pubkey)
  const [tokenVault] = getVaultPda(poolPubkey)
  // reserves can be null only in a tiny race window we shouldn't observe in
  // practice (PoolCreated and the initial reserves write are in the same
  // indexer tx). Default to zero so the type stays honest if it ever does.
  const sol = detail.reserves?.sol_reserve ?? 0
  const tokens = detail.reserves?.token_reserve ?? 0
  const tokenDecimals = 6
  const price = tokens > 0 ? sol / LAMPORTS_PER_SOL / (tokens / 10 ** tokenDecimals) : 0
  return {
    address: detail.pool.pubkey,
    config: detail.pool.config,
    tokenMint: detail.pool.token_mint,
    tokenVault: tokenVault.toBase58(),
    lpMint: detail.pool.lp_mint,
    initialSol: detail.pool.sol_initial,
    initialTokens: detail.pool.tokens_initial,
    solReserve: sol,
    tokenReserve: tokens,
    price,
  }
}

// ============================================================================
// Pool detail — used by getters.ts as the indexer-first path
// ============================================================================

// Returns null when the indexer responds 404 (pool genuinely doesn't exist).
// Other failures throw, so the caller's `withFallback` can fall through to RPC.
export async function getPoolDetailFromIndexer(
  indexer: string,
  poolPubkey: string,
): Promise<PoolState | null> {
  try {
    const detail = await indexerFetch<IndexerPoolDetail>(indexer, `/api/pools/${poolPubkey}`)
    return detailToPoolState(detail)
  } catch (e) {
    if ((e as Error).message?.includes('HTTP 404')) return null
    throw e
  }
}

// ============================================================================
// Indexer-only public readers
// ============================================================================

export async function getSwapHistory(query: SwapHistoryQuery): Promise<SwapRow[]> {
  const params = new URLSearchParams()
  if (query.poolId != null) params.set('pool_id', String(query.poolId))
  if (query.user) params.set('user', query.user)
  if (query.tokenMint) params.set('token_mint', query.tokenMint)
  if (query.since) params.set('since', query.since.toISOString())
  if (query.before) params.set('before', query.before.toISOString())
  if (query.limit != null) params.set('limit', String(query.limit))
  const qs = params.toString()
  return indexerFetch<SwapRow[]>(query.indexer, qs ? `/api/swaps?${qs}` : '/api/swaps')
}

export async function getLiquidityHistory(query: LiquidityHistoryQuery): Promise<LiquidityRow[]> {
  const params = new URLSearchParams()
  if (query.poolId != null) params.set('pool_id', String(query.poolId))
  if (query.provider) params.set('provider', query.provider)
  if (query.tokenMint) params.set('token_mint', query.tokenMint)
  if (query.isAdd != null) params.set('is_add', String(query.isAdd))
  if (query.since) params.set('since', query.since.toISOString())
  if (query.before) params.set('before', query.before.toISOString())
  if (query.limit != null) params.set('limit', String(query.limit))
  const qs = params.toString()
  return indexerFetch<LiquidityRow[]>(query.indexer, qs ? `/api/liquidity?${qs}` : '/api/liquidity')
}
