export interface PoolState {
  address: string
  config: string
  tokenMint: string
  tokenVault: string
  lpMint: string
  initialSol: number
  initialTokens: number
  solReserve: number
  tokenReserve: number
  price: number // SOL per token (display units)
}

export interface SwapQuote {
  amountIn: number
  amountOut: number
  fee: number
  priceImpactPercent: number
  buy: boolean
}

export interface IndexerPoolRow {
  pool_id: number
  pubkey: string
  config: string
  token_mint: string
  lp_mint: string
  creator: string
  sol_initial: number
  tokens_initial: number
  lp_supply_initial: number
  slot: number
  signature: string
  created_at: string
}

export interface IndexerReservesRow {
  reserve_id: number
  pool_id: number
  sol_reserve: number
  token_reserve: number
  lp_supply: number
  last_slot: number
  signature: string
  inner_ix_idx: number
  created_at: string
}

export interface IndexerPoolDetail {
  pool: IndexerPoolRow
  reserves: IndexerReservesRow | null
}

export interface SwapRow {
  swap_id: number
  pool_id: number
  user_pk: string
  sol_source: string
  is_buy: boolean
  amount_in_gross: number
  amount_in_net: number
  amount_out_gross: number
  amount_out_net: number
  fee: number
  sol_reserve_after: number
  token_reserve_after: number
  slot: number
  signature: string
  inner_ix_idx: number
  created_at: string
}

export interface LiquidityRow {
  liquidity_id: number
  pool_id: number
  provider: string
  is_add: boolean
  sol_amount_gross: number
  sol_amount_net: number
  tokens_amount_gross: number
  tokens_amount_net: number
  lp_user_amount: number
  lp_locked: number
  lp_supply_after: number
  slot: number
  signature: string
  inner_ix_idx: number
  created_at: string
}

export interface LiquidityHistoryQuery {
  indexer: string
  poolId?: number
  provider?: string
  tokenMint?: string
  isAdd?: boolean
  since?: Date
  before?: Date
  limit?: number
}

export interface SwapHistoryQuery {
  indexer: string
  poolId?: number
  user?: string
  tokenMint?: string
  since?: Date
  before?: Date
  limit?: number
}
