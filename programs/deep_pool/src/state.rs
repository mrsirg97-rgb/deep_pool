use anchor_lang::prelude::*;

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
    // Total swaps executed.
    pub total_swaps: u64,
    // PDA bump seed.
    pub bump: u8,
}

impl Pool {
    pub const LEN: usize = 8   // discriminator
        + 32  // config
        + 32  // token_mint
        + 32  // token_vault
        + 32  // lp_mint
        + 8   // initial_sol
        + 8   // initial_tokens
        + 8   // total_swaps
        + 1; // bump

    // SOL reserve = PDA lamports minus rent-exempt minimum.
    pub fn sol_reserve(pool_info: &AccountInfo) -> Result<u64> {
        let rent = Rent::get()?;
        let rent_exempt = rent.minimum_balance(Self::LEN);
        Ok(pool_info.lamports().saturating_sub(rent_exempt))
    }
}
