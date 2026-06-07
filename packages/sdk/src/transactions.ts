import { BN, BorshCoder, Idl } from '@coral-xyz/anchor'
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  getAssociatedTokenAddressSync,
  TOKEN_2022_PROGRAM_ID,
} from '@solana/spl-token'
import {
  Connection,
  PublicKey,
  SystemProgram,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from '@solana/web3.js'
import idl from './deep_pool.json'
import { PROGRAM_ID } from './constants'
import { getEventAuthorityPda, getLpMintPda, getPoolPda, getVaultPda } from './pda'

const coder = new BorshCoder(idl as unknown as Idl)
const [EVENT_AUTHORITY] = getEventAuthorityPda()

// Compile raw instructions into a signable v0 VersionedTransaction. Each
// `build*Transaction` wraps the matching `build*Instructions` so standalone
// callers get a ready-to-sign tx, while composers (e.g. the torch migration /
// vault-swap path) take the instructions and build their own tx — adding
// memos, priority fees, or batching across programs.
const toVersioned = async (
  connection: Connection,
  instructions: TransactionInstruction[],
  payer: PublicKey,
): Promise<VersionedTransaction> => {
  const { blockhash } = await connection.getLatestBlockhash()
  const message = new TransactionMessage({
    payerKey: payer,
    recentBlockhash: blockhash,
    instructions,
  }).compileToV0Message()
  return new VersionedTransaction(message)
}

// ── create_pool ────────────────────────────────────────────────────────────

export const buildCreatePoolInstructions = async (
  _connection: Connection,
  params: {
    creator: string
    config: string // signer-verified namespace (wallet pubkey for standalone, program PDA for CPI)
    tokenMint: string
    initialTokenAmount: number
    initialSolAmount: number
    // Optional SOL source for the pool's initial liquidity. Defaults to `creator`
    // for wallet callers; a protocol integrator (e.g. torch migration) passes a
    // distinct system-owned PDA so the SOL never transits a user wallet.
    solSource?: string
  },
): Promise<{ instructions: TransactionInstruction[]; pool: string; lpMint: string }> => {
  const creator = new PublicKey(params.creator)
  const solSource = params.solSource ? new PublicKey(params.solSource) : creator
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
      solSource,
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

  return { instructions: [ix], pool: pool.toBase58(), lpMint: lpMint.toBase58() }
}

export const buildCreatePoolTransaction = async (
  connection: Connection,
  params: Parameters<typeof buildCreatePoolInstructions>[1],
): Promise<{ transaction: VersionedTransaction; pool: string; lpMint: string }> => {
  const { instructions, pool, lpMint } = await buildCreatePoolInstructions(connection, params)
  const transaction = await toVersioned(connection, instructions, new PublicKey(params.creator))
  return { transaction, pool, lpMint }
}

// ── swap ──────────────────────────────────────────────────────────────────

export const buildSwapInstructions = async (
  _connection: Connection,
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
): Promise<TransactionInstruction[]> => {
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
  // Swap ix no longer inits the user ATA — prepend an idempotent create.
  // Cheap when the ATA exists; creates it on the cold path. Required because
  // the program-side `init_if_needed` was dropped (CPI callers with program-
  // owned authorities couldn't fund rent via system_program anyway).
  const createUserAta = createAssociatedTokenAccountIdempotentInstruction(
    user, // payer
    userTokenAccount,
    user, // owner
    tokenMint,
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

  return [createUserAta, ix]
}

export const buildSwapTransaction = async (
  connection: Connection,
  params: Parameters<typeof buildSwapInstructions>[1],
): Promise<{ transaction: VersionedTransaction; message: string }> => {
  const instructions = await buildSwapInstructions(connection, params)
  const transaction = await toVersioned(connection, instructions, new PublicKey(params.user))
  const direction = params.buy ? 'Buy' : 'Sell'
  return { transaction, message: `${direction} swap on DeepPool` }
}

// ── add_liquidity ───────────────────────────────────────────────────────────

export const buildAddLiquidityInstructions = async (
  _connection: Connection,
  params: {
    provider: string
    config: string
    tokenMint: string
    tokenAmount: number
    maxSolAmount: number
    minLpOut: number
  },
): Promise<TransactionInstruction[]> => {
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

  return [ix]
}

export const buildAddLiquidityTransaction = async (
  connection: Connection,
  params: Parameters<typeof buildAddLiquidityInstructions>[1],
): Promise<{ transaction: VersionedTransaction; message: string }> => {
  const instructions = await buildAddLiquidityInstructions(connection, params)
  const transaction = await toVersioned(connection, instructions, new PublicKey(params.provider))
  return { transaction, message: 'Add liquidity to DeepPool' }
}

// ── remove_liquidity ────────────────────────────────────────────────────────

export const buildRemoveLiquidityInstructions = async (
  _connection: Connection,
  params: {
    provider: string
    config: string
    tokenMint: string
    lpAmount: number
    minSolOut: number
    minTokensOut: number
  },
): Promise<TransactionInstruction[]> => {
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

  return [ix]
}

export const buildRemoveLiquidityTransaction = async (
  connection: Connection,
  params: Parameters<typeof buildRemoveLiquidityInstructions>[1],
): Promise<{ transaction: VersionedTransaction; message: string }> => {
  const instructions = await buildRemoveLiquidityInstructions(connection, params)
  const transaction = await toVersioned(connection, instructions, new PublicKey(params.provider))
  return { transaction, message: 'Remove liquidity from DeepPool' }
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
