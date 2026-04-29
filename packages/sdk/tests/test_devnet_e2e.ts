/**
 * DeepPool Devnet E2E
 *
 * Submits the four ix paths on Solana devnet and verifies each one flows
 * end-to-end through the indexer:
 *
 *   chain → Helius Laserstream → decoder → writer → Postgres → API
 *
 * Run:
 *   # terminal 1: docker compose up postgres
 *   # terminal 2: cd indexer && cargo run -- run
 *   # terminal 3:
 *   pnpm --filter deeppoolsdk test:devnet
 *
 * Requirements:
 *   - Devnet wallet (~/.config/solana/id.json) with ≥2 SOL
 *   - Indexer running locally, listening on Helius devnet Laserstream
 *   - Postgres up and migrated (compose handles it on first boot)
 *
 * INDEXER_URL env var overrides the API target (default http://localhost:8080).
 */

import {
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  Transaction,
} from '@solana/web3.js'
import {
  TOKEN_2022_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createMint,
  getAssociatedTokenAddressSync,
  mintTo,
} from '@solana/spl-token'
import {
  buildAddLiquidityTransaction,
  buildCreatePoolTransaction,
  buildRemoveLiquidityTransaction,
  buildSwapTransaction,
  getLpMintPda,
  getPoolPda,
} from '../src/index'
import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'

// ============================================================================
// Config
// ============================================================================

const DEVNET_RPC = 'https://api.devnet.solana.com'
const INDEXER_URL = process.env.INDEXER_URL || 'http://localhost:8080'
const WALLET_PATH = path.join(os.homedir(), '.config/solana/id.json')
const TOKEN_DECIMALS = 6
const TOKEN_MULTIPLIER = 10 ** TOKEN_DECIMALS

// Budget for one full test run (rent + pool deposit + swap fees + headroom).
const MIN_WALLET_SOL = 2

// How long to wait for the indexer to ingest each tx. Devnet slot ≈ 400ms,
// Laserstream hop ≈ 50-200ms, indexer write ≈ tens of ms. ~3s typical;
// 30s is generous slack for first-boot warmup and devnet variability.
const INDEXER_TIMEOUT_MS = 30_000

// ============================================================================
// Helpers
// ============================================================================

const loadWallet = (): Keypair => {
  const raw = JSON.parse(fs.readFileSync(WALLET_PATH, 'utf-8'))
  return Keypair.fromSecretKey(Uint8Array.from(raw))
}

const log = (msg: string) => {
  const ts = new Date().toISOString().substr(11, 8)
  console.log(`[${ts}] ${msg}`)
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

const signAndSend = async (
  connection: Connection,
  wallet: Keypair,
  tx: Transaction,
): Promise<string> => {
  tx.partialSign(wallet)
  const sig = await connection.sendRawTransaction(tx.serialize(), {
    skipPreflight: false,
    preflightCommitment: 'confirmed',
  })
  await connection.confirmTransaction(sig, 'confirmed')
  return sig
}

// Poll the indexer API until predicate matches or we time out.
const waitForIndexer = async <T>(
  pathname: string,
  predicate: (rows: T[]) => boolean,
): Promise<T[]> => {
  const start = Date.now()
  let lastErr: any = null
  while (Date.now() - start < INDEXER_TIMEOUT_MS) {
    try {
      const resp = await fetch(`${INDEXER_URL}${pathname}`)
      if (resp.ok) {
        const rows: T[] = await resp.json()
        if (predicate(rows)) return rows
      } else {
        lastErr = `HTTP ${resp.status}`
      }
    } catch (e) {
      lastErr = e
    }
    await sleep(500)
  }
  throw new Error(
    `indexer timeout after ${INDEXER_TIMEOUT_MS}ms on ${pathname}` +
      (lastErr ? ` (last error: ${lastErr})` : ''),
  )
}

// ============================================================================
// Main
// ============================================================================

const main = async () => {
  console.log('='.repeat(60))
  console.log('DeepPool Devnet E2E')
  console.log('='.repeat(60))

  const connection = new Connection(DEVNET_RPC, 'confirmed')
  const wallet = loadWallet()
  log(`wallet:  ${wallet.publicKey.toBase58()}`)
  log(`indexer: ${INDEXER_URL}`)

  const balance = await connection.getBalance(wallet.publicKey)
  log(`balance: ${(balance / LAMPORTS_PER_SOL).toFixed(4)} SOL`)
  if (balance < MIN_WALLET_SOL * LAMPORTS_PER_SOL) {
    throw new Error(
      `wallet needs ≥${MIN_WALLET_SOL} SOL on devnet — try \`solana airdrop 2 --url devnet\``,
    )
  }

  let passed = 0
  let failed = 0
  const ok = (name: string, detail?: string) => {
    passed++
    log(`  ✓ ${name}${detail ? ` — ${detail}` : ''}`)
  }
  const fail = (name: string, err: any) => {
    failed++
    log(`  ✗ ${name} — ${err.message || err}`)
  }

  // ------------------------------------------------------------------
  // 0. Sanity: indexer is reachable
  // ------------------------------------------------------------------
  log('\n[0] Indexer health check')
  try {
    const resp = await fetch(`${INDEXER_URL}/healthz`)
    if (!resp.ok) throw new Error(`status ${resp.status}`)
    const body = (await resp.text()).trim()
    if (body !== 'ok') throw new Error(`unexpected body: ${body}`)
    ok('indexer reachable')
  } catch (e: any) {
    fail('indexer reachable', e)
    log('Is the indexer running? Try: cd indexer && cargo run -- run')
    process.exit(1)
  }

  // ------------------------------------------------------------------
  // 1. Create Token-2022 mint + mint tokens to wallet
  // ------------------------------------------------------------------
  log('\n[1] Create Token-2022 Mint')
  let mint: PublicKey
  try {
    mint = await createMint(
      connection,
      wallet,
      wallet.publicKey,
      null,
      TOKEN_DECIMALS,
      undefined,
      { commitment: 'confirmed' },
      TOKEN_2022_PROGRAM_ID,
    )
    ok('create mint', `mint=${mint.toBase58().slice(0, 8)}...`)
  } catch (e: any) {
    fail('create mint', e)
    process.exit(1)
  }

  const walletAta = getAssociatedTokenAddressSync(
    mint,
    wallet.publicKey,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  try {
    const ataIx = createAssociatedTokenAccountInstruction(
      wallet.publicKey,
      walletAta,
      wallet.publicKey,
      mint,
      TOKEN_2022_PROGRAM_ID,
    )
    const ataTx = new Transaction().add(ataIx)
    const { blockhash } = await connection.getLatestBlockhash()
    ataTx.recentBlockhash = blockhash
    ataTx.feePayer = wallet.publicKey
    await signAndSend(connection, wallet, ataTx)

    await mintTo(
      connection,
      wallet,
      mint,
      walletAta,
      wallet.publicKey,
      500_000_000 * TOKEN_MULTIPLIER,
      [],
      { commitment: 'confirmed' },
      TOKEN_2022_PROGRAM_ID,
    )
    ok('mint tokens', '500M to wallet')
  } catch (e: any) {
    fail('mint tokens', e)
    process.exit(1)
  }

  // ------------------------------------------------------------------
  // 2. Create pool + verify on indexer
  // ------------------------------------------------------------------
  log('\n[2] Create Pool')
  const initialSol = 1 * LAMPORTS_PER_SOL
  const initialTokens = 10_000_000 * TOKEN_MULTIPLIER
  const [poolPda] = getPoolPda(wallet.publicKey, mint)
  let poolSig: string

  try {
    const result = await buildCreatePoolTransaction(connection, {
      creator: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      initialTokenAmount: initialTokens,
      initialSolAmount: initialSol,
    })
    poolSig = await signAndSend(connection, wallet, result.transaction)
    ok('create pool', `sig=${poolSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('create pool', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  log('  waiting for indexer to ingest...')
  try {
    const pools = await waitForIndexer<any>(
      `/api/pools?token_mint=${mint.toBase58()}`,
      (rows) => rows.some((r) => r.pubkey === poolPda.toBase58()),
    )
    const found = pools.find((r) => r.pubkey === poolPda.toBase58())!
    ok(
      'indexer ingested pool',
      `pool_id=${found.pool_id} sol_initial=${found.sol_initial}`,
    )
  } catch (e: any) {
    fail('indexer ingested pool', e)
  }

  // Pool detail endpoint composes pool + reserves
  try {
    const resp = await fetch(`${INDEXER_URL}/api/pools/${poolPda.toBase58()}`)
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`)
    const detail = await resp.json()
    if (!detail.reserves) throw new Error('reserves missing')
    ok(
      'detail endpoint',
      `sol_reserve=${detail.reserves.sol_reserve} token_reserve=${detail.reserves.token_reserve}`,
    )
  } catch (e: any) {
    fail('detail endpoint', e)
  }

  // ------------------------------------------------------------------
  // 3. Buy swap + verify on indexer
  // ------------------------------------------------------------------
  log('\n[3] Swap: Buy 0.1 SOL → tokens')
  let buySig: string
  try {
    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: 0.1 * LAMPORTS_PER_SOL,
      minimumOut: 0,
      buy: true,
    })
    buySig = await signAndSend(connection, wallet, result.transaction)
    ok('buy swap', `sig=${buySig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('buy swap', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  log('  waiting for indexer to ingest...')
  try {
    const swaps = await waitForIndexer<any>(`/api/swaps?limit=20`, (rows) =>
      rows.some((r) => r.signature === buySig),
    )
    const found = swaps.find((r) => r.signature === buySig)!
    ok(
      'indexer ingested swap',
      `is_buy=${found.is_buy} amount_in=${found.amount_in_gross} amount_out=${found.amount_out_net}`,
    )
  } catch (e: any) {
    fail('indexer ingested swap', e)
  }

  // ------------------------------------------------------------------
  // 4. Add liquidity + verify on indexer
  // ------------------------------------------------------------------
  log('\n[4] Add Liquidity')
  let addSig: string
  try {
    const result = await buildAddLiquidityTransaction(connection, {
      provider: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      tokenAmount: 1_000_000 * TOKEN_MULTIPLIER,
      maxSolAmount: 0.5 * LAMPORTS_PER_SOL,
      minLpOut: 0,
    })
    addSig = await signAndSend(connection, wallet, result.transaction)
    ok('add liquidity', `sig=${addSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('add liquidity', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  log('  waiting for indexer to ingest...')
  try {
    const events = await waitForIndexer<any>(
      `/api/liquidity?limit=20`,
      (rows) => rows.some((r) => r.signature === addSig),
    )
    const found = events.find((r) => r.signature === addSig)!
    ok(
      'indexer ingested liquidity add',
      `is_add=${found.is_add} lp_user_amount=${found.lp_user_amount} lp_locked=${found.lp_locked}`,
    )
  } catch (e: any) {
    fail('indexer ingested liquidity add', e)
  }

  // ------------------------------------------------------------------
  // 5. Remove liquidity + verify on indexer
  // ------------------------------------------------------------------
  log('\n[5] Remove Liquidity (10% of LP)')
  let removeSig: string
  try {
    const [lpMint] = getLpMintPda(poolPda)
    const lpAta = getAssociatedTokenAddressSync(
      lpMint,
      wallet.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const lpBalance = await connection.getTokenAccountBalance(lpAta)
    const lpAmount = Math.floor(Number(lpBalance.value.amount) / 10)

    const result = await buildRemoveLiquidityTransaction(connection, {
      provider: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      lpAmount,
      minSolOut: 0,
      minTokensOut: 0,
    })
    removeSig = await signAndSend(connection, wallet, result.transaction)
    ok('remove liquidity', `sig=${removeSig.slice(0, 8)}... lpAmount=${lpAmount}`)
  } catch (e: any) {
    fail('remove liquidity', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  log('  waiting for indexer to ingest...')
  try {
    const events = await waitForIndexer<any>(
      `/api/liquidity?limit=20`,
      (rows) => rows.some((r) => r.signature === removeSig),
    )
    const found = events.find((r) => r.signature === removeSig)!
    ok(
      'indexer ingested liquidity remove',
      `is_add=${found.is_add} lp_burned=${found.lp_user_amount}`,
    )
  } catch (e: any) {
    fail('indexer ingested liquidity remove', e)
  }

  // ------------------------------------------------------------------
  // 6. Final state via indexer
  // ------------------------------------------------------------------
  log('\n[6] Final pool state via indexer')
  try {
    const resp = await fetch(`${INDEXER_URL}/api/pools/${poolPda.toBase58()}`)
    const detail = await resp.json()
    log(`  pool:     ${detail.pool.pubkey}`)
    log(
      `  reserves: sol=${detail.reserves.sol_reserve} token=${detail.reserves.token_reserve} lp=${detail.reserves.lp_supply}`,
    )
    log(`  last_slot: ${detail.reserves.last_slot}`)
    ok('final state queryable')
  } catch (e: any) {
    fail('final state queryable', e)
  }

  // ------------------------------------------------------------------
  // Results
  // ------------------------------------------------------------------
  console.log('\n' + '='.repeat(60))
  console.log(`RESULTS: ${passed} passed, ${failed} failed`)
  console.log('='.repeat(60))
  process.exit(failed > 0 ? 1 : 0)
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
