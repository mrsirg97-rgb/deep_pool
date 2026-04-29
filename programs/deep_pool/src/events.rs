use anchor_lang::prelude::*;

#[event]
pub struct PoolCreated {
    pub pool: Pubkey,
    pub config: Pubkey,
    pub token_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub creator: Pubkey,
    pub sol_in_gross: u64,
    pub sol_in_net: u64,
    pub tokens_in_gross: u64,
    pub tokens_in_net: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
    pub lp_to_creator: u64,
    pub lp_locked: u64,
}

#[event]
pub struct LiquidityAdded {
    pub pool: Pubkey,
    pub provider: Pubkey,
    pub sol_in_gross: u64,
    pub sol_in_net: u64,
    pub tokens_in_gross: u64,
    pub tokens_in_net: u64,
    pub lp_to_provider: u64,
    pub lp_locked: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
}

#[event]
pub struct LiquidityRemoved {
    pub pool: Pubkey,
    pub provider: Pubkey,
    pub lp_burned: u64,
    pub sol_out_gross: u64,
    pub sol_out_net: u64,
    pub tokens_out_gross: u64,
    pub tokens_out_net: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
    pub lp_supply_after: u64,
}

#[event]
pub struct SwapExecuted {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub sol_source: Pubkey,
    pub buy: bool,
    pub amount_in_gross: u64,
    pub amount_in_net: u64,
    pub amount_out_gross: u64,
    pub amount_out_net: u64,
    pub fee: u64,
    pub sol_reserve_after: u64,
    pub token_reserve_after: u64,
}
