export interface PoolState {
  address: string
  config: string
  tokenMint: string
  tokenVault: string
  lpMint: string
  initialSol: number
  initialTokens: number
  totalSwaps: number
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
