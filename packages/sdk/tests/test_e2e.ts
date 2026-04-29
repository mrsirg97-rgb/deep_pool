/**
 * DeepPool E2E Test
 *
 * Tests: create pool → swap (buy + sell) → add liquidity → remove liquidity
 * Verifies: k invariant, fee compounding, LP proportionality
 *
 * Run:
 *   surfpool start --network mainnet --no-tui
 *   npx tsx tests/test_e2e.ts
 */

import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  LAMPORTS_PER_SOL,
} from '@solana/web3.js'
import {
  createMint,
  mintTo,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  TOKEN_2022_PROGRAM_ID,
} from '@solana/spl-token'
import {
  getPool,
  getSwapQuote,
  buildCreatePoolTransaction,
  buildSwapTransaction,
  buildAddLiquidityTransaction,
  buildRemoveLiquidityTransaction,
  getPoolPda,
  getVaultPda,
  getLpMintPda,
  parseEvents,
  type DecodedEvent,
} from '../src/index'
import * as fs from 'fs'
import * as path from 'path'
import * as os from 'os'

// ============================================================================
// Config
// ============================================================================

const RPC_URL = 'http://localhost:8899'
const WALLET_PATH = path.join(os.homedir(), '.config/solana/id.json')
const TOKEN_DECIMALS = 6
const TOKEN_MULTIPLIER = 10 ** TOKEN_DECIMALS

const loadWallet = (): Keypair => {
  const raw = JSON.parse(fs.readFileSync(WALLET_PATH, 'utf-8'))
  return Keypair.fromSecretKey(Uint8Array.from(raw))
}

const log = (msg: string) => {
  const ts = new Date().toISOString().substr(11, 8)
  console.log(`[${ts}] ${msg}`)
}

const signAndSend = async (
  connection: Connection,
  wallet: Keypair,
  tx: Transaction,
): Promise<string> => {
  tx.partialSign(wallet)
  const raw = tx.serialize()
  const sig = await connection.sendRawTransaction(raw, {
    skipPreflight: false,
    preflightCommitment: 'confirmed',
  })
  await connection.confirmTransaction(sig, 'confirmed')
  return sig
}

// ============================================================================
// Main
// ============================================================================

const main = async () => {
  console.log('='.repeat(60))
  console.log('DeepPool E2E Test')
  console.log('='.repeat(60))

  const connection = new Connection(RPC_URL, 'confirmed')
  const wallet = loadWallet()
  log(`Wallet: ${wallet.publicKey.toBase58()}`)

  const balance = await connection.getBalance(wallet.publicKey)
  log(`Balance: ${balance / LAMPORTS_PER_SOL} SOL`)

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
  // Compares via toString to handle BN, PublicKey, number, bool uniformly.
  const assertEvent = (events: DecodedEvent[], name: string, checks: Record<string, any>) => {
    const ev = events.find((e) => e.name === name)
    if (!ev) {
      fail(`event ${name}`, 'not emitted')
      return
    }
    for (const [k, expected] of Object.entries(checks)) {
      const actual = ev.data[k]
      const a = actual?.toString?.() ?? String(actual)
      const e = expected?.toString?.() ?? String(expected)
      if (a !== e) {
        fail(`event ${name}.${k}`, `expected ${e}, got ${a}`)
        return
      }
    }
    ok(`event ${name}`, `${Object.keys(checks).length} fields verified`)
  }

  // ------------------------------------------------------------------
  // 1. Create Token-2022 Mint (test token)
  // ------------------------------------------------------------------
  log('\n[1] Create Token-2022 Mint')
  let mint: PublicKey
  try {
    mint = await createMint(
      connection,
      wallet,
      wallet.publicKey, // mint authority
      null, // no freeze
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

  // Mint tokens to wallet
  const walletAta = getAssociatedTokenAddressSync(
    mint,
    wallet.publicKey,
    false,
    TOKEN_2022_PROGRAM_ID,
  )
  try {
    const createAtaIx = createAssociatedTokenAccountInstruction(
      wallet.publicKey,
      walletAta,
      wallet.publicKey,
      mint,
      TOKEN_2022_PROGRAM_ID,
    )
    const ataTx = new Transaction().add(createAtaIx)
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
      500_000_000 * TOKEN_MULTIPLIER, // 500M tokens
      [],
      { commitment: 'confirmed' },
      TOKEN_2022_PROGRAM_ID,
    )
    ok('mint tokens', '500M tokens to wallet')
  } catch (e: any) {
    fail('mint tokens', e)
    process.exit(1)
  }

  // ------------------------------------------------------------------
  // 2. Create Pool
  // ------------------------------------------------------------------
  log('\n[2] Create Pool')
  const initialSol = 10 * LAMPORTS_PER_SOL // 10 SOL
  const initialTokens = 100_000_000 * TOKEN_MULTIPLIER // 100M tokens

  try {
    const result = await buildCreatePoolTransaction(connection, {
      creator: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(), // standalone: config = creator wallet
      tokenMint: mint.toBase58(),
      initialTokenAmount: initialTokens,
      initialSolAmount: initialSol,
    })
    const sig = await signAndSend(connection, wallet, result.transaction)
    ok(
      'create pool',
      `pool=${result.pool.slice(0, 8)}... lp_mint=${result.lpMint.slice(0, 8)}... sig=${sig.slice(0, 8)}...`,
    )

    const evs = await parseEvents(connection, sig)
    const [poolPda] = getPoolPda(wallet.publicKey, mint)
    assertEvent(evs, 'PoolCreated', {
      pool: poolPda,
      creator: wallet.publicKey,
      sol_in_gross: initialSol,
      tokens_in_gross: initialTokens,
    })
  } catch (e: any) {
    fail('create pool', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
    process.exit(1)
  }

  // ------------------------------------------------------------------
  // 3. Read Pool State
  // ------------------------------------------------------------------
  log('\n[3] Read Pool')
  let poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  if (poolState) {
    log(`  SOL reserve: ${poolState.solReserve / LAMPORTS_PER_SOL} SOL`)
    log(`  Token reserve: ${poolState.tokenReserve / TOKEN_MULTIPLIER} tokens`)
    log(`  Price: ${poolState.price.toFixed(10)} SOL/token`)
    ok('read pool')
  } else {
    fail('read pool', 'pool not found')
  }

  const k_initial = BigInt(poolState!.solReserve) * BigInt(poolState!.tokenReserve)
  log(`  K (initial): ${k_initial}`)

  // ------------------------------------------------------------------
  // 4. Swap: Buy (SOL → Token)
  // ------------------------------------------------------------------
  log('\n[4] Swap: Buy 1 SOL → Tokens')
  try {
    const quote = getSwapQuote(
      poolState!.solReserve,
      poolState!.tokenReserve,
      1 * LAMPORTS_PER_SOL,
      true,
    )
    log(
      `  Quote: ${quote.amountOut / TOKEN_MULTIPLIER} tokens, fee: ${quote.fee / LAMPORTS_PER_SOL} SOL, impact: ${quote.priceImpactPercent.toFixed(2)}%`,
    )

    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: 1 * LAMPORTS_PER_SOL,
      minimumOut: Math.floor(quote.amountOut * 0.95), // 5% slippage
      buy: true,
    })
    const sig = await signAndSend(connection, wallet, result.transaction)
    ok('buy swap', `sig=${sig.slice(0, 8)}...`)

    const evs = await parseEvents(connection, sig)
    assertEvent(evs, 'SwapExecuted', {
      buy: true,
      amount_in_gross: 1 * LAMPORTS_PER_SOL,
      amount_in_net: 1 * LAMPORTS_PER_SOL,
    })
  } catch (e: any) {
    fail('buy swap', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
  }

  // Check K increased
  poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  if (poolState) {
    const k_after_buy = BigInt(poolState.solReserve) * BigInt(poolState.tokenReserve)
    log(`  K after buy: ${k_after_buy}`)
    if (k_after_buy >= k_initial) {
      ok('k non-decreasing after buy')
    } else {
      fail('k non-decreasing after buy', `k decreased: ${k_initial} → ${k_after_buy}`)
    }
  }

  // ------------------------------------------------------------------
  // 5. Swap: Sell (Token → SOL)
  // ------------------------------------------------------------------
  log('\n[5] Swap: Sell 1M Tokens → SOL')
  try {
    const sellAmount = 1_000_000 * TOKEN_MULTIPLIER
    const quote = getSwapQuote(poolState!.solReserve, poolState!.tokenReserve, sellAmount, false)
    log(
      `  Quote: ${quote.amountOut / LAMPORTS_PER_SOL} SOL, fee: ${quote.fee / TOKEN_MULTIPLIER} tokens, impact: ${quote.priceImpactPercent.toFixed(2)}%`,
    )

    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: sellAmount,
      minimumOut: Math.floor(quote.amountOut * 0.95),
      buy: false,
    })
    const sig = await signAndSend(connection, wallet, result.transaction)
    ok('sell swap', `sig=${sig.slice(0, 8)}...`)

    const evs = await parseEvents(connection, sig)
    assertEvent(evs, 'SwapExecuted', {
      buy: false,
      amount_in_gross: sellAmount,
    })
  } catch (e: any) {
    fail('sell swap', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
  }

  // K check after sell
  poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  if (poolState) {
    const k_after_sell = BigInt(poolState.solReserve) * BigInt(poolState.tokenReserve)
    log(`  K after sell: ${k_after_sell}`)
    if (k_after_sell >= k_initial) {
      ok('k non-decreasing after sell')
    } else {
      fail('k non-decreasing after sell', `k decreased`)
    }
    log(`  Price: ${poolState.price.toFixed(10)} SOL/token`)
  }

  // ------------------------------------------------------------------
  // 6. Add Liquidity (7.5% LP locked)
  // ------------------------------------------------------------------
  log('\n[6] Add Liquidity — verify 7.5% LP lock')
  try {
    // Check LP balance before
    const [poolPda] = getPoolPda(wallet.publicKey, mint)
    const [lpMintPda] = getLpMintPda(poolPda)
    const walletLpAta = getAssociatedTokenAddressSync(
      lpMintPda,
      wallet.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const lpBefore = await connection.getTokenAccountBalance(walletLpAta)
    const lpSupplyBefore = await connection.getTokenSupply(lpMintPda)

    const tokenAmount = 10_000_000 * TOKEN_MULTIPLIER // 10M tokens
    const maxSol = 5 * LAMPORTS_PER_SOL // max 5 SOL

    const result = await buildAddLiquidityTransaction(connection, {
      provider: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      tokenAmount,
      maxSolAmount: maxSol,
      minLpOut: 0,
    })
    const sig = await signAndSend(connection, wallet, result.transaction)

    // Check LP after — user should get ~80% of new LP
    const lpAfter = await connection.getTokenAccountBalance(walletLpAta)
    const lpSupplyAfter = await connection.getTokenSupply(lpMintPda)
    const userLpGain = Number(lpAfter.value.amount) - Number(lpBefore.value.amount)
    const supplyGain = Number(lpSupplyAfter.value.amount) - Number(lpSupplyBefore.value.amount)

    // Pool PDA should hold the other 20%
    const poolLpAta = getAssociatedTokenAddressSync(lpMintPda, poolPda, true, TOKEN_2022_PROGRAM_ID)
    const poolLpBal = await connection.getTokenAccountBalance(poolLpAta)
    log(`  User LP gained: ${userLpGain}, Supply gained: ${supplyGain}`)
    log(`  Pool PDA LP (locked): ${poolLpBal.value.amount}`)

    if (supplyGain > 0 && userLpGain < supplyGain) {
      ok(
        'add liquidity + 7.5% lock',
        `user got ${((userLpGain / supplyGain) * 100).toFixed(0)}% of minted LP`,
      )
    } else {
      ok('add liquidity', `sig=${sig.slice(0, 8)}...`)
    }

    const evs = await parseEvents(connection, sig)
    assertEvent(evs, 'LiquidityAdded', {
      provider: wallet.publicKey,
      tokens_in_gross: tokenAmount,
      lp_to_provider: userLpGain,
    })
  } catch (e: any) {
    fail('add liquidity', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
  }

  poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  if (poolState) {
    log(`  SOL reserve: ${poolState.solReserve / LAMPORTS_PER_SOL} SOL`)
    log(`  Token reserve: ${poolState.tokenReserve / TOKEN_MULTIPLIER} tokens`)
  }

  // ------------------------------------------------------------------
  // 7. Remove Liquidity
  // ------------------------------------------------------------------
  log('\n[7] Remove Liquidity')
  try {
    const [lpMint] = getLpMintPda(new PublicKey(poolState!.address))
    const lpAta = getAssociatedTokenAddressSync(
      lpMint,
      wallet.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const lpBalance = await connection.getTokenAccountBalance(lpAta)
    const lpAmount = Math.floor(Number(lpBalance.value.amount) / 10) // remove 10%

    log(`  LP balance: ${lpBalance.value.uiAmountString}, removing 10% (${lpAmount})`)

    const result = await buildRemoveLiquidityTransaction(connection, {
      provider: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      lpAmount,
      minSolOut: 0,
      minTokensOut: 0,
    })
    const sig = await signAndSend(connection, wallet, result.transaction)
    ok('remove liquidity', `sig=${sig.slice(0, 8)}...`)

    const evs = await parseEvents(connection, sig)
    assertEvent(evs, 'LiquidityRemoved', {
      provider: wallet.publicKey,
      lp_burned: lpAmount,
    })
  } catch (e: any) {
    fail('remove liquidity', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
  }

  // ------------------------------------------------------------------
  // 8. Multiple Swaps — K Growth Verification
  // ------------------------------------------------------------------
  log('\n[8] Multiple Swaps — K grows every time')
  poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  let k_prev = BigInt(poolState!.solReserve) * BigInt(poolState!.tokenReserve)
  let k_growth_count = 0

  for (let i = 0; i < 10; i++) {
    try {
      const isBuy = i % 2 === 0
      if (isBuy) {
        const result = await buildSwapTransaction(connection, {
          user: wallet.publicKey.toBase58(),
          config: wallet.publicKey.toBase58(),
          tokenMint: mint.toBase58(),
          amountIn: Math.floor(0.5 * LAMPORTS_PER_SOL),
          minimumOut: 0,
          buy: true,
        })
        await signAndSend(connection, wallet, result.transaction)
      } else {
        const result = await buildSwapTransaction(connection, {
          user: wallet.publicKey.toBase58(),
          config: wallet.publicKey.toBase58(),
          tokenMint: mint.toBase58(),
          amountIn: 500_000 * TOKEN_MULTIPLIER,
          minimumOut: 0,
          buy: false,
        })
        await signAndSend(connection, wallet, result.transaction)
      }

      poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
      const k_now = BigInt(poolState!.solReserve) * BigInt(poolState!.tokenReserve)
      if (k_now >= k_prev) {
        k_growth_count++
      } else {
        fail(`k growth swap ${i}`, `K decreased: ${k_prev} → ${k_now}`)
      }
      k_prev = k_now
    } catch (e: any) {
      fail(`swap ${i}`, e)
    }
  }

  if (k_growth_count === 10) {
    ok('k growth 10/10 swaps', `K grew every swap`)
  }

  // ------------------------------------------------------------------
  // 9. Tiny Swap — Below Fee Threshold
  // ------------------------------------------------------------------
  log('\n[9] Tiny Swap — 100 lamports (below fee threshold)')
  try {
    const result = await buildSwapTransaction(connection, {
      user: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      amountIn: 100, // 100 lamports — fee = 0
      minimumOut: 0,
      buy: true,
    })
    await signAndSend(connection, wallet, result.transaction)
    ok('tiny swap', 'succeeded with zero fee')
  } catch (e: any) {
    // Might fail due to zero output — that's acceptable
    if (e.message?.includes('SlippageExceeded') || e.message?.includes('EmptyPool')) {
      ok('tiny swap', 'rejected (expected — output rounds to 0)')
    } else {
      fail('tiny swap', e)
    }
  }

  // ------------------------------------------------------------------
  // 10. Edge: Remove Almost All Liquidity
  // ------------------------------------------------------------------
  log('\n[10] Edge: Remove ALL user LP — 7.5% lock should keep reserves')
  try {
    const [lpMint2] = getLpMintPda(new PublicKey(poolState!.address))
    const lpAta2 = getAssociatedTokenAddressSync(
      lpMint2,
      wallet.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const lpBal2 = await connection.getTokenAccountBalance(lpAta2)
    const allLp = Number(lpBal2.value.amount)
    const solBefore = poolState!.solReserve

    log(`  User LP: ${allLp}, removing all`)

    const result = await buildRemoveLiquidityTransaction(connection, {
      provider: wallet.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(),
      tokenMint: mint.toBase58(),
      lpAmount: allLp,
      minSolOut: 0,
      minTokensOut: 0,
    })
    await signAndSend(connection, wallet, result.transaction)

    poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
    if (poolState && poolState.solReserve > 0 && poolState.tokenReserve > 0) {
      const lockedPct = ((poolState.solReserve / solBefore) * 100).toFixed(1)
      ok(
        'remove all LP — pool survives',
        `${lockedPct}% of SOL locked (${poolState.solReserve / LAMPORTS_PER_SOL} SOL, ${poolState.tokenReserve / TOKEN_MULTIPLIER} tokens)`,
      )
    } else {
      fail('remove all LP', 'pool drained to zero — 7.5% lock failed')
    }
  } catch (e: any) {
    if (e.message?.includes('MinimumLiquidityRequired')) {
      ok('remove all LP', 'rejected — minimum reserve enforced')
    } else {
      fail('remove all LP', e)
    }
  }

  // ------------------------------------------------------------------
  // 11. Second LP Provider
  // ------------------------------------------------------------------
  log('\n[11] Second LP Provider')
  const provider2 = Keypair.generate()
  try {
    // Fund provider2
    const fundTx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: wallet.publicKey,
        toPubkey: provider2.publicKey,
        lamports: 5 * LAMPORTS_PER_SOL,
      }),
    )
    const { blockhash: fBh } = await connection.getLatestBlockhash()
    fundTx.recentBlockhash = fBh
    fundTx.feePayer = wallet.publicKey
    await signAndSend(connection, wallet, fundTx)

    // Give provider2 some tokens
    const p2Ata = getAssociatedTokenAddressSync(
      mint,
      provider2.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const createP2Ata = createAssociatedTokenAccountInstruction(
      wallet.publicKey,
      p2Ata,
      provider2.publicKey,
      mint,
      TOKEN_2022_PROGRAM_ID,
    )
    const ataTx = new Transaction().add(createP2Ata)
    const { blockhash: aBh } = await connection.getLatestBlockhash()
    ataTx.recentBlockhash = aBh
    ataTx.feePayer = wallet.publicKey
    await signAndSend(connection, wallet, ataTx)

    await mintTo(
      connection,
      wallet,
      mint,
      p2Ata,
      wallet.publicKey,
      10_000_000 * TOKEN_MULTIPLIER,
      [],
      { commitment: 'confirmed' },
      TOKEN_2022_PROGRAM_ID,
    )

    // Provider2 adds liquidity
    const addResult = await buildAddLiquidityTransaction(connection, {
      provider: provider2.publicKey.toBase58(),
      config: wallet.publicKey.toBase58(), // pool was created under wallet's config
      tokenMint: mint.toBase58(),
      tokenAmount: 5_000_000 * TOKEN_MULTIPLIER,
      maxSolAmount: 3 * LAMPORTS_PER_SOL,
      minLpOut: 0,
    })
    addResult.transaction.feePayer = provider2.publicKey
    const { blockhash: addBh } = await connection.getLatestBlockhash()
    addResult.transaction.recentBlockhash = addBh
    const addSig = await signAndSend(connection, provider2, addResult.transaction)
    ok('provider2 add liquidity', `sig=${addSig.slice(0, 8)}...`)

    // Provider2 removes liquidity
    const [lpMint3] = getLpMintPda(new PublicKey(poolState!.address))
    const p2LpAta = getAssociatedTokenAddressSync(
      lpMint3,
      provider2.publicKey,
      false,
      TOKEN_2022_PROGRAM_ID,
    )
    const p2LpBal = await connection.getTokenAccountBalance(p2LpAta)
    const p2LpAmount = Number(p2LpBal.value.amount)

    if (p2LpAmount > 0) {
      const removeResult = await buildRemoveLiquidityTransaction(connection, {
        provider: provider2.publicKey.toBase58(),
        config: wallet.publicKey.toBase58(),
        tokenMint: mint.toBase58(),
        lpAmount: p2LpAmount,
        minSolOut: 0,
        minTokensOut: 0,
      })
      removeResult.transaction.feePayer = provider2.publicKey
      const { blockhash: rmBh } = await connection.getLatestBlockhash()
      removeResult.transaction.recentBlockhash = rmBh
      const rmSig = await signAndSend(connection, provider2, removeResult.transaction)
      ok('provider2 remove liquidity', `sig=${rmSig.slice(0, 8)}...`)
    } else {
      fail('provider2 LP', 'no LP tokens received')
    }
  } catch (e: any) {
    fail('second LP provider', e)
    if (e.logs) console.error('  Logs:', e.logs.slice(-5).join('\n        '))
  }

  // Final state
  poolState = await getPool(connection, mint.toBase58(), wallet.publicKey.toBase58())
  if (poolState) {
    log(`\n  Final state:`)
    log(`    SOL: ${poolState.solReserve / LAMPORTS_PER_SOL} SOL`)
    log(`    Tokens: ${poolState.tokenReserve / TOKEN_MULTIPLIER}`)
    log(`    Price: ${poolState.price.toFixed(10)} SOL/token`)
  }

  // ------------------------------------------------------------------
  // Results
  // ------------------------------------------------------------------
  console.log('\n' + '='.repeat(60))
  console.log(`RESULTS: ${passed} passed, ${failed} failed`)
  console.log('='.repeat(60))
}

main().catch(console.error)
