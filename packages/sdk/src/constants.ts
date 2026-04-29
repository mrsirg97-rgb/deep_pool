import { PublicKey } from '@solana/web3.js'
import idl from './deep_pool.json'

export const PROGRAM_ID = new PublicKey(idl.address)
export const POOL_SEED = Buffer.from('deep_pool')
export const VAULT_SEED = Buffer.from('pool_vault')
export const LP_MINT_SEED = Buffer.from('pool_lp_mint')
export const EVENT_AUTHORITY_SEED = Buffer.from('__event_authority')
export const SWAP_FEE_BPS = 25
export const FEE_DENOMINATOR = 10000

// Pool::LEN from programs/deep_pool/src/state.rs.
// 8 disc + 32×4 pubkeys (config, token_mint, token_vault, lp_mint)
// + 8+8 u64 (initial_sol, initial_tokens) + 1 u8 (bump) = 153.
export const POOL_ACCOUNT_SIZE = 153
