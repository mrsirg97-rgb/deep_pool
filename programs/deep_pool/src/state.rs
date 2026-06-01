use anchor_lang::prelude::*;

use crate::constants::TWAP_RING_SIZE;

// One TWAP ring snapshot. `cum_sol_per_tok` / `cum_tok_per_sol` are wrapping
// running sums of `price_q64 × slot_delta` (Q64.64 prices, both directions). A
// window's time-weighted price is `(cum[newest] − cum[oldest]) / Δslot` via
// wrapping_sub. See docs/twap-oracle.md.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default)]
pub struct Observation {
    pub slot: u64,
    pub cum_sol_per_tok: u128,
    pub cum_tok_per_sol: u128,
}

impl Observation {
    pub const LEN: usize = 8 + 16 + 16; // 40 bytes
}

#[account]
pub struct Pool {
    // Namespace config — signer-verified at creation, prevents pool squatting.
    pub config: Pubkey,
    // The Token-2022 mint this pool trades against SOL.
    pub token_mint: Pubkey,
    // Token-2022 vault ATA (owned by this PDA).
    pub token_vault: Pubkey,
    // LP token mint (mint authority = this PDA).
    pub lp_mint: Pubkey,
    // SOL deposited at creation (immutable reference).
    pub initial_sol: u64,
    // Net tokens deposited at creation (immutable reference).
    pub initial_tokens: u64,
    // PDA bump seed.
    pub bump: u8,
    // --- TWAP oracle (keeperless; advanced on every swap) ---
    // Live Q64.64 price-cumulative head (both directions), advanced from
    // `last_cum_slot` using pre-swap reserves. Wrapping (mod 2^128).
    pub cum_sol_per_tok: u128,
    pub cum_tok_per_sol: u128,
    pub last_cum_slot: u64,
    // Periodic snapshots of the head (written at MIN_OBS_SPACING_SLOTS cadence).
    pub observations: [Observation; TWAP_RING_SIZE],
    pub obs_head: u16,
}

impl Pool {
    pub const LEN: usize = 8   // discriminator
        + 32  // config
        + 32  // token_mint
        + 32  // token_vault
        + 32  // lp_mint
        + 8   // initial_sol
        + 8   // initial_tokens
        + 1   // bump
        + 16  // cum_sol_per_tok
        + 16  // cum_tok_per_sol
        + 8   // last_cum_slot
        + (Observation::LEN * TWAP_RING_SIZE) // observations
        + 2; // obs_head

    // SOL reserve = PDA lamports minus rent-exempt minimum.
    pub fn sol_reserve(pool_info: &AccountInfo) -> Result<u64> {
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(Self::LEN);
        Ok(pool_info.lamports().saturating_sub(rent_exempt))
    }

    // Seed the oracle clock at pool creation (no interval to accumulate yet).
    pub fn init_oracle(&mut self, now: u64) {
        self.cum_sol_per_tok = 0;
        self.cum_tok_per_sol = 0;
        self.last_cum_slot = now;
        self.observations = [Observation::default(); TWAP_RING_SIZE];
        self.obs_head = 0;
    }

    fn newest_obs(&self) -> Observation {
        let ring = TWAP_RING_SIZE;
        let idx = if self.obs_head == 0 {
            ring - 1
        } else {
            self.obs_head as usize - 1
        };
        self.observations[idx]
    }

    fn write_observation(&mut self, now: u64) {
        let head = self.obs_head as usize;
        self.observations[head] = Observation {
            slot: now,
            cum_sol_per_tok: self.cum_sol_per_tok,
            cum_tok_per_sol: self.cum_tok_per_sol,
        };
        self.obs_head = ((head + 1) % TWAP_RING_SIZE) as u16;
    }

    // Advance the oracle from the PRE-swap reserves. Called at the top of every
    // swap (the price held over `[last_cum_slot, now]` was these reserves). The
    // live head advances every call; a ring snapshot is written only once per
    // MIN_OBS_SPACING_SLOTS so the lookback span is swap-frequency-independent.
    // Dust pools (< MIN_SPOT_RESERVE) advance the clock but don't accumulate —
    // their marks aren't trustworthy. See docs/twap-oracle.md.
    pub fn record_observation(
        &mut self,
        sol_reserve: u64,
        token_reserve: u64,
        now: u64,
    ) -> Result<()> {
        if now <= self.last_cum_slot {
            return Ok(()); // same slot / clock not advanced (idempotent)
        }
        if sol_reserve < crate::constants::MIN_SPOT_RESERVE {
            self.last_cum_slot = now; // skip accumulation, don't mis-weight the gap
            return Ok(());
        }
        let slot_delta = now - self.last_cum_slot;
        // Q64.64 prices of the interval that just ended (pre-swap reserves);
        // accumulate both directions, wrapping. price_q64 is None only on a zero
        // reserve, excluded by the MIN_SPOT_RESERVE / EmptyPool gates upstream.
        let sol_per_tok = crate::math::price_q64(sol_reserve, token_reserve)
            .ok_or(crate::error::DeepPoolError::MathOverflow)?;
        let tok_per_sol = crate::math::price_q64(token_reserve, sol_reserve)
            .ok_or(crate::error::DeepPoolError::MathOverflow)?;
        self.cum_sol_per_tok =
            crate::math::accumulate_price(self.cum_sol_per_tok, sol_per_tok, slot_delta);
        self.cum_tok_per_sol =
            crate::math::accumulate_price(self.cum_tok_per_sol, tok_per_sol, slot_delta);
        self.last_cum_slot = now;
        if now.saturating_sub(self.newest_obs().slot) >= crate::constants::MIN_OBS_SPACING_SLOTS {
            self.write_observation(now);
        }
        Ok(())
    }

    // Time-weighted SOL-per-token price over the ring window, as Q64.64 (the
    // mark torch values token debt/collateral against). Lazily extends the head
    // to `now` using the current reserves (price is constant since the last
    // swap). `None` until the oldest observation is ≥ one spacing old (warmup) →
    // consumers fail closed. The reverse direction lives in `cum_tok_per_sol`.
    pub fn read_twap_sol_per_tok(
        &self,
        sol_reserve_now: u64,
        token_reserve_now: u64,
        now: u64,
        lookback_slots: u64,
    ) -> Option<u128> {
        let gap = now.saturating_sub(self.last_cum_slot);
        let price_now = crate::math::price_q64(sol_reserve_now, token_reserve_now)?;
        let cum_now = crate::math::accumulate_price(self.cum_sol_per_tok, price_now, gap);

        // Window start = the newest observation at least `lookback_slots` old
        // (largest slot ≤ now − lookback). The window is therefore ≥ lookback (and
        // ≤ lookback + spacing). `None` if the ring lacks that much history (pool
        // younger than the requested window) → the caller fails closed. Window
        // length is the *consumer's* policy; spacing/ring are just storage.
        let target = now.saturating_sub(lookback_slots);
        let mut start: Option<Observation> = None;
        for obs in self.observations.iter() {
            if obs.slot == 0 || obs.slot > target {
                continue;
            }
            match start {
                None => start = Some(*obs),
                Some(s) if obs.slot > s.slot => start = Some(*obs),
                _ => {}
            }
        }
        let start = start?;
        let dt = now.saturating_sub(start.slot);
        if dt == 0 {
            return None;
        }
        // Wrapping difference = true window accumulation (exact while < 2^128);
        // ÷ elapsed slots → time-weighted Q64.64 SOL-per-token price. The read
        // intentionally tracks the *current* price over the gap (lazy head-extension),
        // including on a sub-floor / crashed pool — a position underwater at a held
        // crashed mark MUST stay liquidatable (bad debt can't sit behind a depth gate).
        // A read-side MIN_SPOT_RESERVE floor was considered and rejected for exactly
        // this reason; the consumer owns its own liquidity-floor policy (see I-8).
        let cum_delta = cum_now.wrapping_sub(start.cum_sol_per_tok);
        cum_delta.checked_div(dt as u128)
    }
}
