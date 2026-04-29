/**
 * DeepPool Local Stack E2E
 *
 * Submits txs to Solana devnet, then exercises the SDK's indexer-first
 * read path against a local docker-compose indexer. Validates the full
 * loop:
 *
 *   chain (devnet) → Helius Laserstream → local indexer → Postgres
 *                                                              ↓
 *                       SDK getPool({ indexer }) → /api/pools/:pubkey
 *
 * Also exercises the indexer-only readers (`getSwapHistory`,
 * `getLiquidityHistory`) which have no RPC equivalent.
 *
 * Run:
 *   # terminal 1: bring up the docker compose stack
 *   docker compose --profile full up --build
 *
 *   # terminal 2:
 *   pnpm --filter deeppoolsdk test:local
 *
 * Requirements:
 *   - Devnet wallet (~/.config/solana/id.json) with ≥2 SOL
 *   - Docker compose stack running (postgres + indexer + app)
 *   - Indexer's `LASERSTREAM_*` env vars configured for devnet
 *
 * INDEXER_URL env var overrides the indexer target (default http://localhost:8080).
 */

import { Connection, Keypair, LAMPORTS_PER_SOL, PublicKey, Transaction } from '@solana/web3.js'
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
  getLiquidityHistory,
  getLpMintPda,
  getPool,
  getPoolByAddress,
  getPoolPda,
  getSwapHistory,
} from '../src/index'
import type { PoolState } from '../src/index'
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

const MIN_WALLET_SOL = 2
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

// Poll the indexer-first SDK read until predicate matches or we time out.
// Lets us assert on the SDK's view of indexer state, not just raw indexer HTTP.
const waitForIndexerRead = async <T>(
  read: () => Promise<T | null>,
  predicate: (value: T) => boolean,
): Promise<T> => {
  const start = Date.now()
  let lastErr: any = null
  while (Date.now() - start < INDEXER_TIMEOUT_MS) {
    try {
      const value = await read()
      if (value && predicate(value)) return value
    } catch (e) {
      lastErr = e
    }
    await sleep(500)
  }
  throw new Error(
    `indexer read timeout after ${INDEXER_TIMEOUT_MS}ms` +
      (lastErr ? ` (last error: ${lastErr})` : ''),
  )
}

// ============================================================================
// Main
// ============================================================================

const main = async () => {
  console.log('='.repeat(60))
  console.log('DeepPool Local Stack E2E')
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

  // Generic agreement check between indexer-first and RPC reads. They should
  // converge once the indexer has caught up.
  const assertReadsAgree = (name: string, fromIndexer: PoolState, fromRpc: PoolState) => {
    const fields: (keyof PoolState)[] = [
      'address',
      'config',
      'tokenMint',
      'tokenVault',
      'lpMint',
      'initialSol',
      'initialTokens',
      'solReserve',
      'tokenReserve',
    ]
    for (const f of fields) {
      if (fromIndexer[f] !== fromRpc[f]) {
        fail(name, `field ${String(f)} mismatch: indexer=${fromIndexer[f]} rpc=${fromRpc[f]}`)
        return
      }
    }
    ok(name, `${fields.length} fields match between indexer and rpc`)
  }

  // ------------------------------------------------------------------
  // 0. Indexer health
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
    log('Is the local stack up? Try: docker compose --profile full up --build')
    process.exit(1)
  }

  // ------------------------------------------------------------------
  // 1. Mint setup
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
  // 2. Create pool — verify via SDK indexer-first read
  // ------------------------------------------------------------------
  log('\n[2] Create Pool + indexer-first read')
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

  // SDK indexer-first read — should hit /api/pools/:pubkey, return after ingest
  log('  awaiting indexer ingest via SDK getPool({ indexer })...')
  let poolFromIndexer: PoolState
  try {
    poolFromIndexer = await waitForIndexerRead(
      () =>
        getPool(connection, mint.toBase58(), wallet.publicKey.toBase58(), {
          indexer: INDEXER_URL,
        }),
      (p) => p.solReserve === initialSol,
    )
    ok(
      'getPool({ indexer }) returns ingested pool',
      `sol=${poolFromIndexer.solReserve} tokens=${poolFromIndexer.tokenReserve}`,
    )
  } catch (e: any) {
    fail('getPool({ indexer })', e)
    process.exit(1)
  }

  // RPC read — should match indexer-first since both are post-tx-confirmed
  try {
    const poolFromRpc = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
    if (!poolFromRpc) throw new Error('rpc returned null')
    assertReadsAgree('getPool indexer ↔ rpc', poolFromIndexer, poolFromRpc)
  } catch (e: any) {
    fail('getPool rpc-vs-indexer compare', e)
  }

  // getPoolByAddress with indexer too
  try {
    const byAddr = await getPoolByAddress(connection, poolPda, {
      indexer: INDEXER_URL,
    })
    if (!byAddr) throw new Error('returned null')
    if (byAddr.address !== poolPda.toBase58()) throw new Error('address mismatch')
    ok('getPoolByAddress({ indexer })', `address=${byAddr.address.slice(0, 8)}...`)
  } catch (e: any) {
    fail('getPoolByAddress({ indexer })', e)
  }

  // ------------------------------------------------------------------
  // 3. Buy swap → verify via getSwapHistory
  // ------------------------------------------------------------------
  log('\n[3] Buy swap + getSwapHistory')
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

  log('  awaiting indexer ingest via getSwapHistory({ indexer })...')
  try {
    const swaps = await (async () => {
      const start = Date.now()
      while (Date.now() - start < INDEXER_TIMEOUT_MS) {
        const rows = await getSwapHistory({ indexer: INDEXER_URL, limit: 20 })
        if (rows.some((r) => r.signature === buySig)) return rows
        await sleep(500)
      }
      throw new Error('timeout waiting for swap')
    })()
    const swap = swaps.find((r) => r.signature === buySig)!
    ok(
      'getSwapHistory returns the buy',
      `is_buy=${swap.is_buy} amount_in=${swap.amount_in_gross} amount_out=${swap.amount_out_net}`,
    )
  } catch (e: any) {
    fail('getSwapHistory', e)
  }

  // ------------------------------------------------------------------
  // 4. Add liquidity → verify via getLiquidityHistory(isAdd=true)
  // ------------------------------------------------------------------
  log('\n[4] Add liquidity + getLiquidityHistory(isAdd=true)')
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

  log('  awaiting indexer ingest via getLiquidityHistory({ indexer, isAdd: true })...')
  try {
    const events = await (async () => {
      const start = Date.now()
      while (Date.now() - start < INDEXER_TIMEOUT_MS) {
        const rows = await getLiquidityHistory({
          indexer: INDEXER_URL,
          isAdd: true,
          limit: 20,
        })
        if (rows.some((r) => r.signature === addSig)) return rows
        await sleep(500)
      }
      throw new Error('timeout waiting for liquidity add')
    })()
    const event = events.find((r) => r.signature === addSig)!
    if (!event.is_add) throw new Error('expected is_add=true filter')
    ok(
      'getLiquidityHistory(isAdd=true) returns the add',
      `lp_user_amount=${event.lp_user_amount} lp_locked=${event.lp_locked}`,
    )
  } catch (e: any) {
    fail('getLiquidityHistory(isAdd=true)', e)
  }

  // ------------------------------------------------------------------
  // 5. Remove liquidity → verify via getLiquidityHistory(isAdd=false)
  // ------------------------------------------------------------------
  log('\n[5] Remove liquidity + getLiquidityHistory(isAdd=false)')
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
    ok('remove liquidity', `sig=${removeSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('remove liquidity', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  log('  awaiting indexer ingest via getLiquidityHistory({ indexer, isAdd: false })...')
  try {
    const events = await (async () => {
      const start = Date.now()
      while (Date.now() - start < INDEXER_TIMEOUT_MS) {
        const rows = await getLiquidityHistory({
          indexer: INDEXER_URL,
          isAdd: false,
          limit: 20,
        })
        if (rows.some((r) => r.signature === removeSig)) return rows
        await sleep(500)
      }
      throw new Error('timeout waiting for liquidity remove')
    })()
    const event = events.find((r) => r.signature === removeSig)!
    if (event.is_add) throw new Error('expected is_add=false filter')
    if (event.lp_locked !== 0) throw new Error('expected lp_locked=0 on remove')
    ok(
      'getLiquidityHistory(isAdd=false) returns the remove',
      `lp_burned=${event.lp_user_amount} lp_locked=${event.lp_locked}`,
    )
  } catch (e: any) {
    fail('getLiquidityHistory(isAdd=false)', e)
  }

  // ------------------------------------------------------------------
  // 6. Final SDK indexer-first read — assert post-state visible
  // ------------------------------------------------------------------
  log('\n[6] Final indexer-first read of pool state')
  try {
    const final = await getPoolByAddress(connection, poolPda, {
      indexer: INDEXER_URL,
    })
    if (!final) throw new Error('null')
    log(
      `  sol_reserve=${final.solReserve} token_reserve=${final.tokenReserve} price=${final.price.toFixed(10)}`,
    )
    ok('post-mutation state visible via indexer')
  } catch (e: any) {
    fail('post-mutation indexer read', e)
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
