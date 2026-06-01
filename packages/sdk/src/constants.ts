import { PublicKey } from '@solana/web3.js'
import idl from './deep_pool.json'

export const PROGRAM_ID = new PublicKey(idl.address)
export const TOKEN_2022_PROGRAM_ID = new PublicKey('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb')
export const POOL_SEED = Buffer.from('deep_pool')
export const VAULT_SEED = Buffer.from('pool_vault')
export const LP_MINT_SEED = Buffer.from('pool_lp_mint')
export const EVENT_AUTHORITY_SEED = Buffer.from('__event_authority')
export const SWAP_FEE_BPS = 25
export const FEE_DENOMINATOR = 10000
// Spot-reserve floor (mirrors deep_pool MIN_SPOT_RESERVE = 5 SOL). Below this the
// pool's live price is untrustworthy; the TWAP read fails closed (returns null),
// matching the on-chain write+read gate.
export const MIN_SPOT_RESERVE = 5_000_000_000n
// Pool::LEN from programs/deep_pool/src/state.rs.
// 8 disc + 32×4 pubkeys (config, token_mint, token_vault, lp_mint)
// + 8+8 u64 (initial_sol, initial_tokens) + 1 u8 (bump) = 153 base
// + TWAP oracle: 16 cum_sol_per_tok + 16 cum_tok_per_sol + 8 last_cum_slot
//   + 40×16 observations + 2 obs_head = 682 → 835.
export const POOL_ACCOUNT_SIZE = 835
// TWAP oracle: min slots a window must span before the mark is valid (mirrors
// deep_pool MIN_OBS_SPACING_SLOTS). read returns null until the oldest
// observation is at least this old (warmup → fail closed).
export const MIN_OBS_SPACING_SLOTS = 500
export const LAMPORTS_PER_SOL = 1_000_000_000
