import { BN, BorshCoder, Idl } from '@coral-xyz/anchor'
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_2022_PROGRAM_ID,
} from '@solana/spl-token'
import {
  Connection,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from '@solana/web3.js'
import idl from './deep_pool.json'
import { PROGRAM_ID } from './constants'
import { getEventAuthorityPda, getLpMintPda, getPoolPda, getVaultPda } from './pda'

const coder = new BorshCoder(idl as unknown as Idl)
const [EVENT_AUTHORITY] = getEventAuthorityPda()

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
  const creatorTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    creator,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const creatorLpAccount = getAssociatedTokenAddressSync(
    lpMint,
    creator,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const poolLpAccount = getAssociatedTokenAddressSync(lpMint, pool, true, TOKEN_2022_PROGRAM_ID)
  const ix = await buildInstruction(
    'create_pool',
    {
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
      eventAuthority: EVENT_AUTHORITY,
      program: PROGRAM_ID,
    },
    {
      args: {
        initial_token_amount: new BN(params.initialTokenAmount),
        initial_sol_amount: new BN(params.initialSolAmount),
      },
    },
  )

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
    // Optional SOL source/sink. Defaults to `user` for wallet callers.
    // CPI-style callers pass a distinct system-owned PDA.
    solSource?: string
  },
): Promise<{ transaction: Transaction; message: string }> => {
  const user = new PublicKey(params.user)
  const solSource = params.solSource ? new PublicKey(params.solSource) : user
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const userTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    user,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const ix = await buildInstruction(
    'swap',
    {
      user,
      solSource,
      pool,
      tokenMint,
      tokenVault: vault,
      userTokenAccount,
      tokenProgram: TOKEN_2022_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      eventAuthority: EVENT_AUTHORITY,
      program: PROGRAM_ID,
    },
    {
      args: {
        amount_in: new BN(params.amountIn),
        minimum_out: new BN(params.minimumOut),
        buy: params.buy,
      },
    },
  )

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
    minLpOut: number
  },
): Promise<{ transaction: Transaction; message: string }> => {
  const provider = new PublicKey(params.provider)
  const config = new PublicKey(params.config)
  const tokenMint = new PublicKey(params.tokenMint)
  const [pool] = getPoolPda(config, tokenMint)
  const [vault] = getVaultPda(pool)
  const [lpMint] = getLpMintPda(pool)
  const providerTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    provider,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const providerLpAccount = getAssociatedTokenAddressSync(
    lpMint,
    provider,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const poolLpAccount = getAssociatedTokenAddressSync(lpMint, pool, true, TOKEN_2022_PROGRAM_ID)
  const ix = await buildInstruction(
    'add_liquidity',
    {
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
      eventAuthority: EVENT_AUTHORITY,
      program: PROGRAM_ID,
    },
    {
      args: {
        token_amount: new BN(params.tokenAmount),
        max_sol_amount: new BN(params.maxSolAmount),
        min_lp_out: new BN(params.minLpOut),
      },
    },
  )

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
  const providerTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    provider,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const providerLpAccount = getAssociatedTokenAddressSync(
    lpMint,
    provider,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  const ix = await buildInstruction(
    'remove_liquidity',
    {
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
      eventAuthority: EVENT_AUTHORITY,
      program: PROGRAM_ID,
    },
    {
      args: {
        lp_amount: new BN(params.lpAmount),
        min_sol_out: new BN(params.minSolOut),
        min_tokens_out: new BN(params.minTokensOut),
      },
    },
  )

  const tx = new Transaction().add(ix)
  tx.feePayer = provider
  const { blockhash } = await connection.getLatestBlockhash()
  tx.recentBlockhash = blockhash
  return { transaction: tx, message: 'Remove liquidity from DeepPool' }
}

async function buildInstruction(
  name: string,
  accounts: Record<string, PublicKey>,
  args: Record<string, any>,
): Promise<TransactionInstruction> {
  const ix = (coder.instruction as any).encode(name, args)
  const keys = Object.entries(accounts).map(([key, pubkey]) => {
    const isSigner =
      key === 'creator' ||
      key === 'provider' ||
      key === 'user' ||
      key === 'solSource' ||
      key === 'config'
    const isWritable =
      key !== 'tokenProgram' &&
      key !== 'associatedTokenProgram' &&
      key !== 'systemProgram' &&
      key !== 'tokenMint' &&
      key !== 'config' &&
      key !== 'eventAuthority' &&
      key !== 'program'
    return { pubkey, isSigner, isWritable }
  })

  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys,
    data: ix,
  })
}
