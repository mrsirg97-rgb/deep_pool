use anchor_lang::prelude::*;

#[error_code]
pub enum DeepPoolError {
    #[msg("Initial SOL deposit below minimum")]
    InsufficientInitialSol,

    #[msg("Initial token deposit below minimum")]
    InsufficientInitialTokens,

    #[msg("Token mint must be Token-2022")]
    NotToken2022,

    #[msg("Swap output below minimum (slippage exceeded)")]
    SlippageExceeded,

    #[msg("Swap input must be greater than zero")]
    ZeroInput,

    #[msg("Pool reserves are empty")]
    EmptyPool,

    #[msg("Arithmetic overflow")]
    MathOverflow,

    #[msg("SOL required exceeds maximum (slippage exceeded)")]
    SolSlippageExceeded,

    #[msg("Insufficient LP token balance")]
    InsufficientLpTokens,

    #[msg("Cannot remove all liquidity (minimum reserve)")]
    MinimumLiquidityRequired,

    #[msg("Token deposit must be greater than zero")]
    ZeroDeposit,

    #[msg("SOL output below minimum (slippage exceeded)")]
    SolOutputSlippage,

    #[msg("Token output below minimum (slippage exceeded)")]
    TokenOutputSlippage,
}
