import { BorshCoder, Idl } from '@coral-xyz/anchor'
import { Connection, LAMPORTS_PER_SOL, PublicKey } from '@solana/web3.js'
import idl from './deep_pool.json'
import { FEE_DENOMINATOR, POOL_ACCOUNT_SIZE, PROGRAM_ID, SWAP_FEE_BPS } from './constants'
import { PoolState, SwapQuote } from './types'
import { getPoolPda } from './pda'
import { getPoolDetailFromIndexer } from './indexer'

// Read-time options. Pass `indexer` to try the indexer-first path; on any
// failure (network, non-200, malformed response) the call silently falls
// back to RPC, so callers always get a result if either source succeeds.
export interface ReadOptions {
  indexer?: string
}

// Generic indexer-first wrapper. Try `primary`; on throw, run `fallback`.
async function withFallback<T>(primary: () => Promise<T>, fallback: () => Promise<T>): Promise<T> {
  try {
    return await primary()
  } catch {
    return fallback()
  }
}

// ============================================================================
// Public getters — indexer-first when configured, RPC otherwise
// ============================================================================

export const getPool = async (
  connection: Connection,
  tokenMint: string,
  config: string,
  options?: ReadOptions,
): Promise<PoolState | null> => {
  if (options?.indexer) {
    const [poolPda] = getPoolPda(new PublicKey(config), new PublicKey(tokenMint))
    return withFallback(
      () => getPoolDetailFromIndexer(options.indexer!, poolPda.toBase58()),
      () => getPoolFromRpc(connection, tokenMint, config),
    )
  }
  return getPoolFromRpc(connection, tokenMint, config)
}

export const getPoolByAddress = async (
  connection: Connection,
  poolAddress: PublicKey,
  options?: ReadOptions,
): Promise<PoolState | null> => {
  if (options?.indexer) {
    return withFallback(
      () => getPoolDetailFromIndexer(options.indexer!, poolAddress.toBase58()),
      () => getPoolByAddressFromRpc(connection, poolAddress),
    )
  }
  return getPoolByAddressFromRpc(connection, poolAddress)
}

export const getPoolsForMint = async (
  connection: Connection,
  tokenMint: string,
): Promise<PoolState[]> => {
  // RPC-only for now: the indexer's list endpoint doesn't return reserves
  // inline, so an indexer-first path here would be N+1 hops. Acceptable
  // for v1; revisit if a `/api/pools/with_reserves` endpoint is added.
  const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
    filters: [{ dataSize: POOL_ACCOUNT_SIZE }, { memcmp: { offset: 40, bytes: tokenMint } }],
  })

  const pools: PoolState[] = []
  for (const { pubkey } of accounts) {
    const pool = await getPoolByAddressFromRpc(connection, pubkey)
    if (pool) pools.push(pool)
  }
  return pools.sort((a, b) => b.solReserve - a.solReserve)
}

// ============================================================================
// RPC-path helpers (the original implementations, factored for reuse)
// ============================================================================

async function getPoolFromRpc(
  connection: Connection,
  tokenMint: string,
  config: string,
): Promise<PoolState | null> {
  const mint = new PublicKey(tokenMint)
  const configPk = new PublicKey(config)
  const [poolPda] = getPoolPda(configPk, mint)
  const coder = new BorshCoder(idl as unknown as Idl)
  const info = await connection.getAccountInfo(poolPda)
  if (!info) return null
  const pool = coder.accounts.decode('Pool', info.data) as any
  if (!pool) return null
  const rent = await connection.getMinimumBalanceForRentExemption(info.data.length)
  const solReserve = info.lamports - rent
  const vaultBalance = await connection.getTokenAccountBalance(new PublicKey(pool.token_vault))
  const tokenReserve = Number(vaultBalance.value.amount)
  const tokenDecimals = 6
  const price =
    tokenReserve > 0 ? solReserve / LAMPORTS_PER_SOL / (tokenReserve / 10 ** tokenDecimals) : 0
  return {
    address: poolPda.toBase58(),
    config,
    tokenMint,
    tokenVault: pool.token_vault.toBase58(),
    lpMint: pool.lp_mint.toBase58(),
    initialSol: Number(pool.initial_sol.toString()),
    initialTokens: Number(pool.initial_tokens.toString()),
    solReserve,
    tokenReserve,
    price,
  }
}

async function getPoolByAddressFromRpc(
  connection: Connection,
  poolAddress: PublicKey,
): Promise<PoolState | null> {
  const coder = new BorshCoder(idl as unknown as Idl)
  const info = await connection.getAccountInfo(poolAddress)
  if (!info) return null
  const pool = coder.accounts.decode('Pool', info.data) as any
  if (!pool) return null
  const rent = await connection.getMinimumBalanceForRentExemption(info.data.length)
  const solReserve = info.lamports - rent
  const vaultBalance = await connection.getTokenAccountBalance(new PublicKey(pool.token_vault))
  const tokenReserve = Number(vaultBalance.value.amount)
  const tokenDecimals = 6
  const price =
    tokenReserve > 0 ? solReserve / LAMPORTS_PER_SOL / (tokenReserve / 10 ** tokenDecimals) : 0
  return {
    address: poolAddress.toBase58(),
    config: pool.config.toBase58(),
    tokenMint: pool.token_mint.toBase58(),
    tokenVault: pool.token_vault.toBase58(),
    lpMint: pool.lp_mint.toBase58(),
    initialSol: Number(pool.initial_sol.toString()),
    initialTokens: Number(pool.initial_tokens.toString()),
    solReserve,
    tokenReserve,
    price,
  }
}

// ============================================================================
// Pure compute (no IO)
// ============================================================================

export const getSwapQuote = (
  solReserve: number,
  tokenReserve: number,
  amountIn: number,
  buy: boolean,
  transferFeeBps: number = 0,
): SwapQuote => {
  if (solReserve <= 0 || tokenReserve <= 0 || amountIn <= 0) {
    return { amountIn, amountOut: 0, fee: 0, priceImpactPercent: 0, buy }
  }

  if (buy) {
    const fee = Math.floor((amountIn * SWAP_FEE_BPS) / FEE_DENOMINATOR)
    const effectiveIn = amountIn - fee
    const tokensOut = Math.floor((effectiveIn * tokenReserve) / (solReserve + effectiveIn))
    const transferFee = Math.floor((tokensOut * transferFeeBps) / FEE_DENOMINATOR)
    const netOut = tokensOut - transferFee
    const spotPrice = solReserve / tokenReserve
    const execPrice = tokensOut > 0 ? amountIn / tokensOut : 0
    const impact = spotPrice > 0 ? (Math.abs(execPrice - spotPrice) / spotPrice) * 100 : 0
    return { amountIn, amountOut: netOut, fee, priceImpactPercent: impact, buy }
  } else {
    const transferFee = Math.floor((amountIn * transferFeeBps) / FEE_DENOMINATOR)
    const netIn = amountIn - transferFee
    const fee = Math.floor((netIn * SWAP_FEE_BPS) / FEE_DENOMINATOR)
    const effectiveIn = netIn - fee
    const solOut = Math.floor((effectiveIn * solReserve) / (tokenReserve + effectiveIn))
    const spotPrice = solReserve / tokenReserve
    const execPrice = amountIn > 0 ? solOut / amountIn : 0
    const impact = spotPrice > 0 ? (Math.abs(execPrice - spotPrice) / spotPrice) * 100 : 0
    return { amountIn, amountOut: solOut, fee, priceImpactPercent: impact, buy }
  }
}
