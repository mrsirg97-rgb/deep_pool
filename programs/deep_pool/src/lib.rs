pub mod constants;
pub mod error;
pub mod instructions;
pub mod math;
pub mod state;

#[cfg(kani)]
mod kani_proofs;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT");

#[program]
pub mod deep_pool {
    use super::*;

    pub fn create_pool(ctx: Context<CreatePool>, args: CreatePoolArgs) -> Result<()> {
        instructions::create_pool::handler(ctx, args)
    }

    pub fn add_liquidity(ctx: Context<AddLiquidity>, args: AddLiquidityArgs) -> Result<()> {
        instructions::add_liquidity::handler(ctx, args)
    }

    pub fn remove_liquidity(
        ctx: Context<RemoveLiquidity>,
        args: RemoveLiquidityArgs,
    ) -> Result<()> {
        instructions::remove_liquidity::handler(ctx, args)
    }

    pub fn swap(ctx: Context<Swap>, args: SwapArgs) -> Result<()> {
        instructions::swap::handler(ctx, args)
    }
}
