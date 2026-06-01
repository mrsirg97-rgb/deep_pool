use anchor_lang::prelude::*;

pub const POOL_SEED: &[u8] = b"deep_pool";
pub const VAULT_SEED: &[u8] = b"pool_vault";
pub const LP_MINT_SEED: &[u8] = b"pool_lp_mint";
pub const SWAP_FEE_BPS: u64 = 25; // 0.25%
pub const FEE_DENOMINATOR: u64 = 10000;
// Minimum LP tokens locked on first deposit to prevent rounding attacks.
pub const MIN_LIQUIDITY: u64 = 1000;
// Minimum initial SOL deposit (prevents dust pools).
pub const MIN_INITIAL_SOL: u64 = 100_000_000; // 0.1 SOL
                                              // Minimum initial token deposit.
pub const MIN_INITIAL_TOKENS: u64 = 1_000_000; // 1 token (6 decimals)
                                               // LP lock rates: creator locks more, community LPs lock less.
pub const LP_LOCK_CREATOR_BPS: u64 = 2000; // 20% on create_pool
pub const LP_LOCK_PROVIDER_BPS: u64 = 750; // 7.5% on add_liquidity

// --- TWAP oracle (in-pool, keeperless — see docs/twap-oracle.md) ---
// Ring of periodic cumulative snapshots; window ≈ TWAP_RING_SIZE × spacing.
pub const TWAP_RING_SIZE: usize = 16;
// Min slots between ring snapshots. The live head advances every swap; the ring
// only records at this cadence, so the lookback span is frequency-independent.
pub const MIN_OBS_SPACING_SLOTS: u64 = 500;
// Liquidity floor: pools thinner than this don't update the oracle (dust pools
// produce manipulable marks). Mirrors torch MIN_SPOT_POOL_SOL.
pub const MIN_SPOT_RESERVE: u64 = 5_000_000_000; // 5 SOL (mirrors torch MIN_POOL_SOL_LENDING)
                                           // Token-2022 program ID.
pub const TOKEN_2022_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77,
    131, 185, 13, 39, 254, 189, 249, 40, 216, 161, 139, 252,
]);
