/**
 * Unit tests for the TWAP read helper — pure, no network.
 *
 * Locks the parts that look right and aren't: the u128 WRAPPING arithmetic
 * (a window that wraps past 2^128 must still difference exactly), the lazy
 * head-extension to `now`, and the warmup fail-closed gate.
 *
 * Run: npx tsx tests/test_twap.ts
 */

import { readTwapSolPerTok, q64ToFloat, TwapObservation, TwapPoolState } from '../src/twap'
import { MIN_OBS_SPACING_SLOTS } from '../src/constants'

const MASK_128 = (1n << 128n) - 1n
const Q64 = 1n << 64n
// price_q64(10 SOL, 1e12 token base units) — the litesvm pool's price.
const P = (10_000_000_000n << 64n) / 1_000_000_000_000n
const SOL = 10_000_000_000n
const TOK = 1_000_000_000_000n

let pass = 0
let fail = 0
const eq = (name: string, got: bigint | null, want: bigint | null) => {
  if (got === want) {
    pass++
    console.log(`ok   - ${name}`)
  } else {
    fail++
    console.error(`FAIL - ${name}\n       got ${got}, want ${want}`)
  }
}
const ok = (name: string, cond: boolean) => {
  if (cond) {
    pass++
    console.log(`ok   - ${name}`)
  } else {
    fail++
    console.error(`FAIL - ${name}`)
  }
}

const EMPTY: TwapObservation = { slot: 0n, cumSolPerTok: 0n, cumTokPerSol: 0n }
const obs = (slot: bigint, cum: bigint): TwapObservation => ({
  slot,
  cumSolPerTok: cum,
  cumTokPerSol: 0n,
})
// Pad a list of observations to the 16-slot ring with empties.
const ring = (set: TwapObservation[]): TwapObservation[] => {
  const r = [...set]
  while (r.length < 16) r.push(EMPTY)
  return r
}
const state = (cumHead: bigint, lastCumSlot: bigint, set: TwapObservation[]): TwapPoolState => ({
  cumSolPerTok: cumHead,
  lastCumSlot,
  observations: ring(set),
})

// All windows below use lookback = 1000 slots unless noted.
const LB = 1000n

// --- fail-closed / guards ---
eq('no observations → null', readTwapSolPerTok(state(0n, 0n, []), SOL, TOK, 0n, LB), null)
eq(
  'lookback longer than available history → null (only a too-recent obs)',
  readTwapSolPerTok(state(P * 100n, 2000n, [obs(1900n, 0n)]), SOL, TOK, 2000n, 500n),
  null, // target = 1500; obs at 1900 isn't ≥ lookback old → no anchor
)
eq(
  'lookback exceeds the whole ring → null',
  readTwapSolPerTok(state(P * 2900n, 3000n, [obs(2900n, P * 2900n)]), SOL, TOK, 3000n, 5000n),
  null,
)
eq(
  'guard: zero token reserve → null',
  readTwapSolPerTok(state(P * 1000n, 2000n, [obs(1000n, 0n)]), SOL, 0n, 2000n, LB),
  null,
)
eq(
  'guard: sub-floor SOL reserve → null (mirrors MIN_SPOT_RESERVE read gate)',
  // 4 SOL < 5 SOL floor — same window as the "constant price → P" case, but the
  // untrustworthy live price now fails closed instead of returning P.
  readTwapSolPerTok(state(P * 1000n, 2000n, [obs(1000n, 0n)]), 4_000_000_000n, TOK, 2000n, LB),
  null,
)
eq(
  'guard: degenerate 0 mark → null (Some(0) fail-closed)',
  // cum_delta = 600 − 100 = 500 over dt = 1000 → floors to 0 → null, instead of a
  // bogus "token worthless" mark reaching the consumer.
  readTwapSolPerTok(state(600n, 2000n, [obs(1000n, 100n)]), SOL, TOK, 2000n, LB),
  null,
)

// --- constant price, gap == 0 (no lazy extension) ---
eq(
  'constant price, gap 0 → P',
  readTwapSolPerTok(state(P * 1000n, 2000n, [obs(1000n, 0n)]), SOL, TOK, 2000n, LB),
  P, // target 1000; anchor obs(1000); dt 1000; (P*1000−0)/1000
)

// --- lazy extension (gap > 0): head extended by current price over the gap ---
// head accrued P over [1000,1500]; extend P over [1500,2000]; window [1000,2000] = P*1000.
eq(
  'lazy extension over a quiet gap → P',
  readTwapSolPerTok(state(P * 500n, 1500n, [obs(1000n, 0n)]), SOL, TOK, 2000n, LB),
  P,
)

// --- WRAP past 2^128: a naive (head - anchor) goes negative; wrapSub must hold ---
// anchor.cum = 2^128 − 100, head = 900 ⇒ true window accumulation = 1000, /1000 = 1.
eq(
  'window that wrapped past 2^128 differences exactly',
  readTwapSolPerTok(state(900n, 2000n, [obs(1000n, MASK_128 - 99n)]), SOL, TOK, 2000n, LB),
  1n,
)

// --- lookback picks the NEWEST obs at least `lookback` old (a tighter window) ---
// Genesis-consistent (cum at slot s == P*s). With lookback 900 (target 2100),
// obs(2000) is the newest ≤ target — chosen over obs(1000), regardless of order.
eq(
  'lookback picks the newest obs ≥ lookback old',
  readTwapSolPerTok(
    state(P * 3000n, 3000n, [obs(2000n, P * 2000n), obs(1000n, P * 1000n), obs(3000n, P * 3000n)]),
    SOL,
    TOK,
    3000n,
    900n,
  ),
  P, // window [2000,3000]: (P*3000 − P*2000)/1000 = P
)

// --- q64ToFloat ---
ok('q64ToFloat(2^64) ≈ 1.0', Math.abs(q64ToFloat(Q64) - 1) < 1e-9)
ok('q64ToFloat(2^63) ≈ 0.5', Math.abs(q64ToFloat(Q64 / 2n) - 0.5) < 1e-9)

console.log(`\n${pass} passed, ${fail} failed`)
process.exit(fail > 0 ? 1 : 0)
