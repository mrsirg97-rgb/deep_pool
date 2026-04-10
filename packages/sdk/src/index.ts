import {
  Connection,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  LAMPORTS_PER_SOL,
} from '@solana/web3.js'
import {
  TOKEN_2022_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from '@solana/spl-token'
import { BorshCoder, Idl, BN } from '@coral-xyz/anchor'
import idl from './deep_pool.json'

// ============================================================================
// Constants
// ============================================================================

export const PROGRAM_ID = new PublicKey(idl.address)
const POOL_SEED = Buffer.from('deep_pool')
const VAULT_SEED = Buffer.from('pool_vault')
const LP_MINT_SEED = Buffer.from('pool_lp_mint')
const SWAP_FEE_BPS = 25
const FEE_DENOMINATOR = 10000

// ============================================================================
// PDA Derivation
// ============================================================================

export const getPoolPda = (config: PublicKey, tokenMint: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([POOL_SEED, config.toBuffer(), tokenMint.toBuffer()], PROGRAM_ID)

export const getVaultPda = (pool: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([VAULT_SEED, pool.toBuffer()], PROGRAM_ID)

export const getLpMintPda = (pool: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([LP_MINT_SEED, pool.toBuffer()], PROGRAM_ID)

// ============================================================================
// Types
// ============================================================================

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

// ============================================================================
// Read Functions
// ============================================================================

export const getPool = async (
  connection: Connection,
  tokenMint: string,
  config: string,
): Promise<PoolState | null> => {
  const mint = new PublicKey(tokenMint)
  const configPk = new PublicKey(config)
  const [poolPda] = getPoolPda(configPk, mint)
  const coder = new BorshCoder(idl as unknown as Idl)

  const info = await connection.getAccountInfo(poolPda)
  if (!info) return null

  const pool = coder.accounts.decode('Pool', info.data) as any
  if (!pool) return null

  // SOL reserve = PDA lamports - rent
  const rent = await connection.getMinimumBalanceForRentExemption(info.data.length)
  const solReserve = info.lamports - rent

  // Token reserve = vault balance
  let tokenReserve = 0
  try {
    const vaultBalance = await connection.getTokenAccountBalance(new PublicKey(pool.token_vault))
    tokenReserve = Number(vaultBalance.value.amount)
  } catch {}

  const tokenDecimals = 6
  const price = tokenReserve > 0
    ? (solReserve / LAMPORTS_PER_SOL) / (tokenReserve / 10 ** tokenDecimals)
    : 0

  return {
    address: poolPda.toBase58(),
    config,
    tokenMint,
    tokenVault: pool.token_vault.toBase58(),
    lpMint: pool.lp_mint.toBase58(),
    initialSol: Number(pool.initial_sol.toString()),
    initialTokens: Number(pool.initial_tokens.toString()),
    totalSwaps: Number(pool.total_swaps.toString()),
    solReserve,
    tokenReserve,
    price,
  }
}

// Get pool by on-chain address (when you have the pool PDA already)
export const getPoolByAddress = async (
  connection: Connection,
  poolAddress: PublicKey,
): Promise<PoolState | null> => {
  const coder = new BorshCoder(idl as unknown as Idl)
  const info = await connection.getAccountInfo(poolAddress)
  if (!info) return null

  const pool = coder.accounts.decode('Pool', info.data) as any
  if (!pool) return null

  const rent = await connection.getMinimumBalanceForRentExemption(info.data.length)
  const solReserve = info.lamports - rent

  let tokenReserve = 0
  try {
    const vaultBalance = await connection.getTokenAccountBalance(new PublicKey(pool.token_vault))
    tokenReserve = Number(vaultBalance.value.amount)
  } catch {}

  const tokenDecimals = 6
  const price = tokenReserve > 0
    ? (solReserve / LAMPORTS_PER_SOL) / (tokenReserve / 10 ** tokenDecimals)
    : 0

  return {
    address: poolAddress.toBase58(),
    config: pool.config.toBase58(),
    tokenMint: pool.token_mint.toBase58(),
    tokenVault: pool.token_vault.toBase58(),
    lpMint: pool.lp_mint.toBase58(),
    initialSol: Number(pool.initial_sol.toString()),
    initialTokens: Number(pool.initial_tokens.toString()),
    totalSwaps: Number(pool.total_swaps.toString()),
    solReserve,
    tokenReserve,
    price,
  }
}

// Find all pools for a token mint (returns deepest first)
export const getPoolsForMint = async (
  connection: Connection,
  tokenMint: string,
): Promise<PoolState[]> => {
  const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
    filters: [
      { dataSize: 161 }, // Pool::LEN with config field
      { memcmp: { offset: 40, bytes: tokenMint } }, // token_mint at offset 8 (disc) + 32 (config)
    ],
  })

  const pools: PoolState[] = []
  for (const { pubkey } of accounts) {
    const pool = await getPoolByAddress(connection, pubkey)
    if (pool) pools.push(pool)
  }

  return pools.sort((a, b) => b.solReserve - a.solReserve)
}

// ============================================================================
// Quote Functions (client-side, no on-chain call)
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
    const tokensOut = Math.floor(
      (effectiveIn * tokenReserve) / (solReserve + effectiveIn),
    )
    const transferFee = Math.floor((tokensOut * transferFeeBps) / FEE_DENOMINATOR)
    const netOut = tokensOut - transferFee

    const spotPrice = solReserve / tokenReserve
    const execPrice = tokensOut > 0 ? amountIn / tokensOut : 0
    const impact = spotPrice > 0 ? Math.abs(execPrice - spotPrice) / spotPrice * 100 : 0

    return { amountIn, amountOut: netOut, fee, priceImpactPercent: impact, buy }
  } else {
    const transferFee = Math.floor((amountIn * transferFeeBps) / FEE_DENOMINATOR)
    const netIn = amountIn - transferFee
    const fee = Math.floor((netIn * SWAP_FEE_BPS) / FEE_DENOMINATOR)
    const effectiveIn = netIn - fee
    const solOut = Math.floor(
      (effectiveIn * solReserve) / (tokenReserve + effectiveIn),
    )

    const spotPrice = solReserve / tokenReserve
    const execPrice = amountIn > 0 ? solOut / amountIn : 0
    const impact = spotPrice > 0 ? Math.abs(execPrice - spotPrice) / spotPrice * 100 : 0

    return { amountIn, amountOut: solOut, fee, priceImpactPercent: impact, buy }
  }
}

// ============================================================================
// Transaction Builders
// ============================================================================

const coder = new BorshCoder(idl as unknown as Idl)

export const buildCreatePoolTransaction = async (
  connection: Connection,
  params: {
    creator: string
    config: string // signer-verified namespace (wallet pubkey for standalone, program PDA for CPI)
    tokenMint: string
    initialTokenAmount: number
    initialSolAmount: number
  },
): Promise<{ transaction: Transaction; pool: string; lpMint: string }> => {
  const creator = new PublicKey(params.creator)
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const [lpMint] = getLpMintPda(pool)
  const creatorTokenAccount = getAssociatedTokenAddressSync(tokenMint, creator, false, TOKEN_2022_PROGRAM_ID)
  const creatorLpAccount = getAssociatedTokenAddressSync(lpMint, creator, false, TOKEN_2022_PROGRAM_ID)
  const poolLpAccount = getAssociatedTokenAddressSync(lpMint, pool, true, TOKEN_2022_PROGRAM_ID)

  const ix = await buildInstruction('create_pool', {
    creator,
    config,
    tokenMint,
    pool,
    tokenVault: vault,
    lpMint,
    creatorTokenAccount,
    creatorLpAccount,
    poolLpAccount,
    tokenProgram: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
  }, {
    args: {
      initial_token_amount: new BN(params.initialTokenAmount),
      initial_sol_amount: new BN(params.initialSolAmount),
    },
  })

  const tx = new Transaction().add(ix)
  tx.feePayer = creator
  const { blockhash } = await connection.getLatestBlockhash()
  tx.recentBlockhash = blockhash

  return { transaction: tx, pool: pool.toBase58(), lpMint: lpMint.toBase58() }
}

export const buildSwapTransaction = async (
  connection: Connection,
  params: {
    user: string
    config: string
    tokenMint: string
    amountIn: number
    minimumOut: number
    buy: boolean
  },
): Promise<{ transaction: Transaction; message: string }> => {
  const user = new PublicKey(params.user)
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const userTokenAccount = getAssociatedTokenAddressSync(tokenMint, user, false, TOKEN_2022_PROGRAM_ID)

  const ix = await buildInstruction('swap', {
    user,
    pool,
    tokenMint,
    tokenVault: vault,
    userTokenAccount,
    tokenProgram: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
  }, {
    args: {
      amount_in: new BN(params.amountIn),
      minimum_out: new BN(params.minimumOut),
      buy: params.buy,
    },
  })

  const tx = new Transaction().add(ix)
  tx.feePayer = user
  const { blockhash } = await connection.getLatestBlockhash()
  tx.recentBlockhash = blockhash

  const direction = params.buy ? 'Buy' : 'Sell'
  return { transaction: tx, message: `${direction} swap on DeepPool` }
}

export const buildAddLiquidityTransaction = async (
  connection: Connection,
  params: {
    provider: string
    config: string
    tokenMint: string
    tokenAmount: number
    maxSolAmount: number
  },
): Promise<{ transaction: Transaction; message: string }> => {
  const provider = new PublicKey(params.provider)
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const [lpMint] = getLpMintPda(pool)
  const providerTokenAccount = getAssociatedTokenAddressSync(tokenMint, provider, false, TOKEN_2022_PROGRAM_ID)
  const providerLpAccount = getAssociatedTokenAddressSync(lpMint, provider, false, TOKEN_2022_PROGRAM_ID)
  const poolLpAccount = getAssociatedTokenAddressSync(lpMint, pool, true, TOKEN_2022_PROGRAM_ID)

  const ix = await buildInstruction('add_liquidity', {
    provider,
    pool,
    tokenMint,
    tokenVault: vault,
    lpMint,
    providerTokenAccount,
    providerLpAccount,
    poolLpAccount,
    tokenProgram: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
  }, {
    args: {
      token_amount: new BN(params.tokenAmount),
      max_sol_amount: new BN(params.maxSolAmount),
    },
  })

  const tx = new Transaction().add(ix)
  tx.feePayer = provider
  const { blockhash } = await connection.getLatestBlockhash()
  tx.recentBlockhash = blockhash

  return { transaction: tx, message: 'Add liquidity to DeepPool' }
}

export const buildRemoveLiquidityTransaction = async (
  connection: Connection,
  params: {
    provider: string
    config: string
    tokenMint: string
    lpAmount: number
    minSolOut: number
    minTokensOut: number
  },
): Promise<{ transaction: Transaction; message: string }> => {
  const provider = new PublicKey(params.provider)
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const [lpMint] = getLpMintPda(pool)
  const providerTokenAccount = getAssociatedTokenAddressSync(tokenMint, provider, false, TOKEN_2022_PROGRAM_ID)
  const providerLpAccount = getAssociatedTokenAddressSync(lpMint, provider, false, TOKEN_2022_PROGRAM_ID)

  const ix = await buildInstruction('remove_liquidity', {
    provider,
    pool,
    tokenMint,
    tokenVault: vault,
    lpMint,
    providerTokenAccount,
    providerLpAccount,
    tokenProgram: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
  }, {
    args: {
      lp_amount: new BN(params.lpAmount),
      min_sol_out: new BN(params.minSolOut),
      min_tokens_out: new BN(params.minTokensOut),
    },
  })

  const tx = new Transaction().add(ix)
  tx.feePayer = provider
  const { blockhash } = await connection.getLatestBlockhash()
  tx.recentBlockhash = blockhash

  return { transaction: tx, message: 'Remove liquidity from DeepPool' }
}

// ============================================================================
// Internal
// ============================================================================

async function buildInstruction(
  name: string,
  accounts: Record<string, PublicKey>,
  args: Record<string, any>,
): Promise<TransactionInstruction> {
  const ix = (coder.instruction as any).encode(name, args)
  const keys = Object.entries(accounts).map(([key, pubkey]) => {
    const isSigner = key === 'creator' || key === 'provider' || key === 'user' || key === 'config'
    const isWritable =
      key !== 'tokenProgram' &&
      key !== 'associatedTokenProgram' &&
      key !== 'systemProgram' &&
      key !== 'tokenMint' &&
      key !== 'config'
    return { pubkey, isSigner, isWritable }
  })

  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys,
    data: ix,
  })
}
