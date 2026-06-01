// TWAP oracle read helper — a faithful TS mirror of deep_pool's on-chain
// `Pool::read_twap_sol_per_tok`. All cumulative arithmetic is u128 **wrapping**
// (mod 2^128), exactly as on-chain: a naive BigInt subtraction would go negative
// and produce a garbage mark. See ../../docs/twap-oracle.md.

import { BorshCoder, Idl } from '@coral-xyz/anchor'
import { Connection, PublicKey } from '@solana/web3.js'
import idl from './deep_pool.json'
import { MIN_SPOT_RESERVE } from './constants'

const MASK_128 = (1n << 128n) - 1n
const Q64 = 1n << 64n

// JS bitwise-AND on a negative BigInt yields the correct unsigned mod-2^128
// value, so `(a - b) & MASK_128` is a faithful wrapping_sub.
const wrapMul = (a: bigint, b: bigint): bigint => (a * b) & MASK_128
const wrapAdd = (a: bigint, b: bigint): bigint => (a + b) & MASK_128
const wrapSub = (a: bigint, b: bigint): bigint => (a - b) & MASK_128

export interface TwapObservation {
  slot: bigint
  cumSolPerTok: bigint
  cumTokPerSol: bigint
}

export interface TwapPoolState {
  cumSolPerTok: bigint
  lastCumSlot: bigint
  observations: TwapObservation[]
}

const toBig = (v: { toString(): string }): bigint => BigInt(v.toString())

// Normalize a BorshCoder-decoded Pool (snake_case BN fields) into bigints.
export function twapStateFromDecodedPool(pool: any): TwapPoolState {
  return {
    cumSolPerTok: toBig(pool.cum_sol_per_tok),
    lastCumSlot: toBig(pool.last_cum_slot),
    observations: (pool.observations as any[]).map((o) => ({
      slot: toBig(o.slot),
      cumSolPerTok: toBig(o.cum_sol_per_tok),
      cumTokPerSol: toBig(o.cum_tok_per_sol),
    })),
  }
}

// Time-weighted SOL-per-token price over a caller-chosen `lookbackSlots` window,
// as Q64.64 — or `null` if the ring lacks that much history (pool younger than
// the window → fail closed). Window start = the newest observation at least
// `lookbackSlots` old (so the realized window is ≥ lookback, ≤ lookback +
// spacing). Lazily extends the head to `nowSlot` with the current reserves
// (price is constant since the last swap), then `(cumNow − start.cum) / Δslot`.
// Faithful mirror of the on-chain `Pool::read_twap_sol_per_tok`.
export function readTwapSolPerTok(
  state: TwapPoolState,
  solReserveNow: bigint,
  tokenReserveNow: bigint,
  nowSlot: bigint,
  lookbackSlots: bigint,
): bigint | null {
  if (tokenReserveNow === 0n) return null
  // Sub-floor pool → live price untrustworthy; fail closed (mirrors the on-chain
  // MIN_SPOT_RESERVE read gate).
  if (solReserveNow < MIN_SPOT_RESERVE) return null
  const priceNow = (solReserveNow << 64n) / tokenReserveNow // price_q64
  const gap = nowSlot > state.lastCumSlot ? nowSlot - state.lastCumSlot : 0n
  const cumNow = wrapAdd(state.cumSolPerTok, wrapMul(priceNow, gap))

  const target = nowSlot > lookbackSlots ? nowSlot - lookbackSlots : 0n
  let start: TwapObservation | null = null
  for (const obs of state.observations) {
    if (obs.slot === 0n || obs.slot > target) continue
    if (start === null || obs.slot > start.slot) start = obs
  }
  if (start === null) return null

  const dt = nowSlot > start.slot ? nowSlot - start.slot : 0n
  if (dt === 0n) return null
  // A degenerate 0 mark (true price < 2^-64) reads to consumers as "worthless" —
  // fail closed, mirroring the on-chain Some(0) → None guard.
  const twap = wrapSub(cumNow, start.cumSolPerTok) / dt
  return twap === 0n ? null : twap
}

// Q64.64 → float: price in **lamports per token base unit** (lossy; display
// only). For a human "SOL per whole token", multiply by 10^tokenDecimals and
// divide by LAMPORTS_PER_SOL at the call site.
export function q64ToFloat(q64: bigint): number {
  const int = q64 >> 64n
  const frac = q64 & (Q64 - 1n)
  return Number(int) + Number(frac) / Number(Q64)
}

// High-level: fetch the pool account + current slot and return the Q64.64 mark
// (or `null` during warmup). Reserves are derived the same way the program does
// (lamports − rent for SOL, vault balance for tokens).
export async function getTwapSolPerTok(
  connection: Connection,
  poolAddress: PublicKey | string,
  lookbackSlots: bigint,
): Promise<bigint | null> {
  const pk = typeof poolAddress === 'string' ? new PublicKey(poolAddress) : poolAddress
  const info = await connection.getAccountInfo(pk)
  if (!info) return null
  const coder = new BorshCoder(idl as unknown as Idl)
  const pool = coder.accounts.decode('Pool', info.data) as any
  const state = twapStateFromDecodedPool(pool)

  const rent = await connection.getMinimumBalanceForRentExemption(info.data.length)
  const solReserveNow = BigInt(info.lamports - rent)
  const vault = await connection.getTokenAccountBalance(new PublicKey(pool.token_vault))
  const tokenReserveNow = BigInt(vault.value.amount)
  const nowSlot = BigInt(await connection.getSlot())

  return readTwapSolPerTok(state, solReserveNow, tokenReserveNow, nowSlot, lookbackSlots)
}
