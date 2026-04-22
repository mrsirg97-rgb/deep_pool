use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        self, Burn, Mint as MintInterface, TokenAccount as TokenAccountInterface, TokenInterface,
        TransferChecked,
    },
};

use crate::constants::*;
use crate::error::DeepPoolError;
use crate::math;
use crate::state::Pool;

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct RemoveLiquidityArgs {
    pub lp_amount: u64,
    pub min_sol_out: u64,
    pub min_tokens_out: u64,
}

#[derive(Accounts)]
#[instruction(args: RemoveLiquidityArgs)]
pub struct RemoveLiquidity<'info> {
    #[account(mut)]
    pub provider: Signer<'info>,
    #[account(
        mut,
        seeds = [POOL_SEED, pool.config.as_ref(), pool.token_mint.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, Pool>,
    #[account(address = pool.token_mint)]
    pub token_mint: InterfaceAccount<'info, MintInterface>,
    #[account(mut, address = pool.token_vault)]
    pub token_vault: InterfaceAccount<'info, TokenAccountInterface>,
    #[account(mut, address = pool.lp_mint)]
    pub lp_mint: InterfaceAccount<'info, MintInterface>,
    #[account(
        mut,
        associated_token::mint = token_mint,
        associated_token::authority = provider,
        associated_token::token_program = token_program,
    )]
    pub provider_token_account: InterfaceAccount<'info, TokenAccountInterface>,
    #[account(
        mut,
        associated_token::mint = lp_mint,
        associated_token::authority = provider,
        associated_token::token_program = token_program,
        constraint = provider_lp_account.amount >= args.lp_amount @ DeepPoolError::InsufficientLpTokens,
    )]
    pub provider_lp_account: InterfaceAccount<'info, TokenAccountInterface>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<RemoveLiquidity>, args: RemoveLiquidityArgs) -> Result<()> {
    require!(args.lp_amount > 0, DeepPoolError::ZeroInput);

    let pool_info = ctx.accounts.pool.to_account_info();
    let sol_reserve = Pool::sol_reserve(&pool_info)?;
    let token_reserve = ctx.accounts.token_vault.amount;
    let lp_supply = ctx.accounts.lp_mint.supply;

    require!(
        sol_reserve > 0 && token_reserve > 0,
        DeepPoolError::EmptyPool
    );
    require!(lp_supply > 0, DeepPoolError::EmptyPool);

    // 1. Compute proportional share
    let sol_out = math::calc_lp_redeem(args.lp_amount, sol_reserve, lp_supply)
        .ok_or(DeepPoolError::MathOverflow)?;
    let tokens_out = math::calc_lp_redeem(args.lp_amount, token_reserve, lp_supply)
        .ok_or(DeepPoolError::MathOverflow)?;

    // 2. Slippage checks
    require!(
        sol_out >= args.min_sol_out,
        DeepPoolError::SolOutputSlippage
    );
    require!(
        tokens_out >= args.min_tokens_out,
        DeepPoolError::TokenOutputSlippage
    );

    // 3. Ensure pool retains minimum reserves after removal
    let sol_remaining = sol_reserve
        .checked_sub(sol_out)
        .ok_or(DeepPoolError::MathOverflow)?;
    let tokens_remaining = token_reserve
        .checked_sub(tokens_out)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(
        sol_remaining > 0 && tokens_remaining > 0,
        DeepPoolError::MinimumLiquidityRequired
    );

    // 4. Burn LP tokens from provider
    token_interface::burn(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.lp_mint.to_account_info(),
                from: ctx.accounts.provider_lp_account.to_account_info(),
                authority: ctx.accounts.provider.to_account_info(),
            },
        ),
        args.lp_amount,
    )?;

    // 5. Transfer tokens from vault to provider (CPI — must happen before lamport manipulation)
    let config_key = ctx.accounts.pool.config;
    let mint_key = ctx.accounts.pool.token_mint;
    let pool_seeds = &[
        POOL_SEED,
        config_key.as_ref(),
        mint_key.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.token_vault.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                to: ctx.accounts.provider_token_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        tokens_out,
        ctx.accounts.token_mint.decimals,
    )?;

    // 6. Transfer SOL from pool PDA to provider (direct lamport — must be LAST, after all CPIs)
    ctx.accounts.pool.to_account_info().sub_lamports(sol_out)?;
    ctx.accounts
        .provider
        .to_account_info()
        .add_lamports(sol_out)?;

    Ok(())
}
