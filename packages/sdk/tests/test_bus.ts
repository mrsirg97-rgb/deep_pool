/**
 * DeepPool Bus E2E
 *
 * Submits txs to Solana devnet and verifies the SDK's IndexerBus receives
 * matching frames over WebSocket from the local docker-compose indexer.
 * Validates the live event path:
 *
 *   chain (devnet) → Helius Laserstream → local indexer → post-COMMIT
 *                                                              broadcast
 *                                                                ↓
 *                                          SDK createIndexerBus → WS /events
 *
 * Run:
 *   # terminal 1: bring up the docker compose stack
 *   docker compose --profile full up --build
 *
 *   # terminal 2:
 *   pnpm --filter deeppoolsdk test:bus
 *
 * Requirements:
 *   - Devnet wallet (~/.config/solana/id.json) with ≥2 SOL
 *   - Docker compose stack running (postgres + indexer + app)
 *   - Node 22+ (global WebSocket) OR Node 18-20 with `ws` installed
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
  createIndexerBus,
  getLpMintPda,
  getPoolPda,
} from '../src/index'
import type { BroadcastFrame, IndexerBus } from '../src/index'
import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'

// ============================================================================
// Polyfill — Node <22 needs an explicit WebSocket. Node 22+ has it global.
// ============================================================================

if (typeof (globalThis as any).WebSocket === 'undefined') {
  try {
    // Lazy require so the test doesn't hard-depend on `ws` for newer Node.

    const WS = require('ws')
    ;(globalThis as any).WebSocket = WS
  } catch {
    console.error(
      'WebSocket is not global on this Node and `ws` is not installed.\n' +
        'Either upgrade to Node 22+, or `pnpm add -D ws` in packages/sdk.',
    )
    process.exit(1)
  }
}

// ============================================================================
// Config
// ============================================================================

const DEVNET_RPC = 'https://api.devnet.solana.com'
const INDEXER_URL = process.env.INDEXER_URL || 'http://localhost:8080'
const WALLET_PATH = path.join(os.homedir(), '.config/solana/id.json')
const TOKEN_DECIMALS = 6
const TOKEN_MULTIPLIER = 10 ** TOKEN_DECIMALS

const MIN_WALLET_SOL = 2
const FRAME_TIMEOUT_MS = 30_000
const CONNECT_TIMEOUT_MS = 5_000

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

// Wait until predicate returns true, or throw on timeout.
const waitFor = async (
  predicate: () => boolean,
  timeoutMs: number,
  label: string,
): Promise<void> => {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    if (predicate()) return
    await sleep(50)
  }
  throw new Error(`${label}: timeout after ${timeoutMs}ms`)
}

// ============================================================================
// Main
// ============================================================================

const main = async () => {
  console.log('='.repeat(60))
  console.log('DeepPool Bus E2E')
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
  // 0. Health + bus connect
  // ------------------------------------------------------------------
  log('\n[0] Indexer health + bus connect')
  try {
    const resp = await fetch(`${INDEXER_URL}/healthz`)
    if (!resp.ok) throw new Error(`status ${resp.status}`)
    ok('indexer reachable')
  } catch (e: any) {
    fail('indexer reachable', e)
    log('Is the local stack up? Try: docker compose --profile full up --build')
    process.exit(1)
  }

  let bus: IndexerBus
  let connected = false
  try {
    bus = createIndexerBus(INDEXER_URL)
    bus.onConnectionChange((state) => {
      if (state === 'connected') connected = true
    })
    await waitFor(() => connected, CONNECT_TIMEOUT_MS, 'bus connect')
    ok('bus connected to ws://...')
  } catch (e: any) {
    fail('bus connect', e)
    process.exit(1)
  }

  // Frame collectors per kind. Tests subscribe once and accumulate.
  const collected: Record<BroadcastFrame['kind'], BroadcastFrame[]> = {
    pool: [],
    swap: [],
    liquidity: [],
    reserves: [],
  }
  bus.on('pool', (f) => collected.pool.push(f))
  bus.on('swap', (f) => collected.swap.push(f))
  bus.on('liquidity', (f) => collected.liquidity.push(f))
  bus.on('reserves', (f) => collected.reserves.push(f))

  // ------------------------------------------------------------------
  // 1. Mint setup (no events expected — just baseline)
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
  // 2. Create pool — expect 'pool' + 'reserves' frames
  // ------------------------------------------------------------------
  log('\n[2] Create pool — expect pool + reserves frames')
  const initialSol = 1 * LAMPORTS_PER_SOL
  const initialTokens = 10_000_000 * TOKEN_MULTIPLIER
  const [poolPda] = getPoolPda(wallet.publicKey, mint)
  const poolPubkey = poolPda.toBase58()
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
    ok('create pool tx', `sig=${poolSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('create pool tx', e)
    process.exit(1)
  }

  log('  awaiting pool frame...')
  try {
    await waitFor(
      () => collected.pool.some((f) => f.kind === 'pool' && f.pubkey === poolPubkey),
      FRAME_TIMEOUT_MS,
      'pool frame',
    )
    const f = collected.pool.find((x) => x.kind === 'pool' && x.pubkey === poolPubkey)!
    if (f.kind !== 'pool') throw new Error('wrong kind')
    if (f.config !== wallet.publicKey.toBase58()) throw new Error('config mismatch')
    if (f.token_mint !== mint.toBase58()) throw new Error('token_mint mismatch')
    ok('pool frame received', `pool_id=${f.pool_id} sol_initial=${f.sol_initial}`)
  } catch (e: any) {
    fail('pool frame', e)
  }

  log('  awaiting reserves frame...')
  try {
    await waitFor(
      () => collected.reserves.some((f) => f.kind === 'reserves' && f.signature === poolSig),
      FRAME_TIMEOUT_MS,
      'reserves frame',
    )
    const f = collected.reserves.find((x) => x.signature === poolSig)!
    if (f.kind !== 'reserves') throw new Error('wrong kind')
    if (f.sol_reserve !== initialSol) throw new Error('sol_reserve mismatch')
    ok('reserves frame received', `sol_reserve=${f.sol_reserve} lp_supply=${f.lp_supply}`)
  } catch (e: any) {
    fail('reserves frame', e)
  }

  // ------------------------------------------------------------------
  // 3. Buy swap — expect 'swap' + 'reserves' frames
  // ------------------------------------------------------------------
  log('\n[3] Buy swap — expect swap + reserves frames')
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
    ok('buy swap tx', `sig=${buySig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('buy swap tx', e)
    process.exit(1)
  }

  try {
    await waitFor(
      () => collected.swap.some((f) => f.kind === 'swap' && f.signature === buySig),
      FRAME_TIMEOUT_MS,
      'swap frame',
    )
    const f = collected.swap.find((x) => x.signature === buySig)!
    if (f.kind !== 'swap') throw new Error('wrong kind')
    if (!f.is_buy) throw new Error('expected is_buy=true')
    ok(
      'swap frame received',
      `is_buy=${f.is_buy} amount_in=${f.amount_in_gross} amount_out=${f.amount_out_net}`,
    )
  } catch (e: any) {
    fail('swap frame', e)
  }

  try {
    await waitFor(
      () => collected.reserves.some((f) => f.kind === 'reserves' && f.signature === buySig),
      FRAME_TIMEOUT_MS,
      'reserves-after-swap frame',
    )
    ok('reserves frame received post-swap')
  } catch (e: any) {
    fail('reserves-after-swap frame', e)
  }

  // ------------------------------------------------------------------
  // 4. Add liquidity — expect 'liquidity' (is_add=true) + 'reserves'
  // ------------------------------------------------------------------
  log('\n[4] Add liquidity — expect liquidity (is_add=true) + reserves')
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
    ok('add liquidity tx', `sig=${addSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('add liquidity tx', e)
    process.exit(1)
  }

  try {
    await waitFor(
      () =>
        collected.liquidity.some(
          (f) => f.kind === 'liquidity' && f.signature === addSig && f.is_add,
        ),
      FRAME_TIMEOUT_MS,
      'liquidity-add frame',
    )
    const f = collected.liquidity.find((x) => x.signature === addSig)!
    if (f.kind !== 'liquidity') throw new Error('wrong kind')
    if (!f.is_add) throw new Error('expected is_add=true')
    if (f.lp_locked === 0) throw new Error('expected lp_locked > 0 on add')
    ok('liquidity-add frame received', `lp_user=${f.lp_user_amount} lp_locked=${f.lp_locked}`)
  } catch (e: any) {
    fail('liquidity-add frame', e)
  }

  // ------------------------------------------------------------------
  // 5. Remove liquidity — expect 'liquidity' (is_add=false) + 'reserves'
  // ------------------------------------------------------------------
  log('\n[5] Remove liquidity — expect liquidity (is_add=false) + reserves')
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
    ok('remove liquidity tx', `sig=${removeSig.slice(0, 8)}...`)
  } catch (e: any) {
    fail('remove liquidity tx', e)
    process.exit(1)
  }

  try {
    await waitFor(
      () =>
        collected.liquidity.some(
          (f) => f.kind === 'liquidity' && f.signature === removeSig && !f.is_add,
        ),
      FRAME_TIMEOUT_MS,
      'liquidity-remove frame',
    )
    const f = collected.liquidity.find((x) => x.signature === removeSig)!
    if (f.kind !== 'liquidity') throw new Error('wrong kind')
    if (f.is_add) throw new Error('expected is_add=false')
    if (f.lp_locked !== 0) throw new Error('expected lp_locked=0 on remove')
    ok('liquidity-remove frame received', `lp_burned=${f.lp_user_amount} lp_locked=${f.lp_locked}`)
  } catch (e: any) {
    fail('liquidity-remove frame', e)
  }

  // ------------------------------------------------------------------
  // 6. Unsubscribe behavior — handler must not fire after unsub
  // ------------------------------------------------------------------
  log('\n[6] Unsubscribe — handler should not fire after unsub')
  let postUnsubCount = 0
  const unsub = bus.on('swap', () => {
    postUnsubCount++
  })
  // Fire one swap to confirm subscription works first
  let preUnsubSig: string
  try {
    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: 0.05 * LAMPORTS_PER_SOL,
      minimumOut: 0,
      buy: true,
    })
    preUnsubSig = await signAndSend(connection, wallet, result.transaction)
    await waitFor(
      () => collected.swap.some((f) => f.signature === preUnsubSig),
      FRAME_TIMEOUT_MS,
      'pre-unsub swap',
    )
    if (postUnsubCount === 0) throw new Error('handler never fired pre-unsub')
    ok('pre-unsub handler fired', `count=${postUnsubCount}`)
  } catch (e: any) {
    fail('pre-unsub baseline', e)
  }

  // Unsubscribe the new handler
  unsub()
  const countBefore = postUnsubCount

  // Fire another swap; the unsubscribed handler should NOT see it.
  // The first 'swap' subscription (collected) still fires, so we use that
  // as the proof-of-arrival signal.
  let postUnsubSig: string
  try {
    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: 0.05 * LAMPORTS_PER_SOL,
      minimumOut: 0,
      buy: true,
    })
    postUnsubSig = await signAndSend(connection, wallet, result.transaction)
    await waitFor(
      () => collected.swap.some((f) => f.signature === postUnsubSig),
      FRAME_TIMEOUT_MS,
      'post-unsub swap arrival',
    )
    if (postUnsubCount !== countBefore) {
      throw new Error(`handler fired ${postUnsubCount - countBefore} times after unsub`)
    }
    ok('unsub stops dispatch', `count stable at ${postUnsubCount}`)
  } catch (e: any) {
    fail('unsub stops dispatch', e)
  }

  // ------------------------------------------------------------------
  // 7. Bus close
  // ------------------------------------------------------------------
  log('\n[7] Bus close')
  try {
    bus.close()
    ok('bus closed cleanly')
  } catch (e: any) {
    fail('bus close', e)
  }

  // ------------------------------------------------------------------
  // Results
  // ------------------------------------------------------------------
  console.log('\n' + '='.repeat(60))
  console.log(`RESULTS: ${passed} passed, ${failed} failed`)
  console.log('='.repeat(60))

  // Summary of frames seen
  console.log('\nFrame totals:')
  console.log(`  pool:      ${collected.pool.length}`)
  console.log(`  swap:      ${collected.swap.length}`)
  console.log(`  liquidity: ${collected.liquidity.length}`)
  console.log(`  reserves:  ${collected.reserves.length}`)

  process.exit(failed > 0 ? 1 : 0)
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
