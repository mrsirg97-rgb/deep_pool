use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        self, Mint as MintInterface, TokenAccount as TokenAccountInterface, TokenInterface,
        TransferChecked, MintTo,
    },
};

use crate::constants::*;
use crate::error::DeepPoolError;
use crate::state::Pool;

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct AddLiquidityArgs {
    pub token_amount: u64,
    pub max_sol_amount: u64,
}

#[derive(Accounts)]
#[instruction(args: AddLiquidityArgs)]
pub struct AddLiquidity<'info> {
    #[account(mut)]
    pub provider: Signer<'info>,
    #[account(
        mut,
        seeds = [POOL_SEED, pool.token_mint.as_ref()],
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
        init_if_needed,
        payer = provider,
        associated_token::mint = lp_mint,
        associated_token::authority = provider,
        associated_token::token_program = token_program,
    )]
    pub provider_lp_account: InterfaceAccount<'info, TokenAccountInterface>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<AddLiquidity>, args: AddLiquidityArgs) -> Result<()> {
    require!(args.token_amount > 0, DeepPoolError::ZeroDeposit);

    let pool_info = ctx.accounts.pool.to_account_info();
    let sol_reserve = Pool::sol_reserve(&pool_info)?;
    let token_reserve = ctx.accounts.token_vault.amount;
    require!(sol_reserve > 0 && token_reserve > 0, DeepPoolError::EmptyPool);

    let lp_supply = ctx.accounts.lp_mint.supply;
    require!(lp_supply > 0, DeepPoolError::EmptyPool);

    // 1. Compute required SOL for proportional deposit
    // sol_required = token_amount * sol_reserve / token_reserve
    let sol_required = (args.token_amount as u128)
        .checked_mul(sol_reserve as u128)
        .ok_or(DeepPoolError::MathOverflow)?
        .checked_div(token_reserve as u128)
        .ok_or(DeepPoolError::MathOverflow)? as u64;

    require!(sol_required > 0, DeepPoolError::ZeroDeposit);
    require!(
        sol_required <= args.max_sol_amount,
        DeepPoolError::SolSlippageExceeded
    );

    // 2. Transfer tokens from provider to vault (measure net)
    let vault_before = ctx.accounts.token_vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.provider_token_account.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                to: ctx.accounts.token_vault.to_account_info(),
                authority: ctx.accounts.provider.to_account_info(),
            },
        ),
        args.token_amount,
        ctx.accounts.token_mint.decimals,
    )?;

    ctx.accounts.token_vault.reload()?;
    let net_tokens = ctx.accounts.token_vault.amount
        .checked_sub(vault_before)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(net_tokens > 0, DeepPoolError::ZeroDeposit);

    // 3. Transfer SOL from provider to pool PDA
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.provider.to_account_info(),
                to: pool_info,
            },
        ),
        sol_required,
    )?;

    // 4. Compute LP tokens to mint (proportional to token deposit)
    // lp_amount = lp_supply * net_tokens / token_reserve
    let lp_amount = (lp_supply as u128)
        .checked_mul(net_tokens as u128)
        .ok_or(DeepPoolError::MathOverflow)?
        .checked_div(token_reserve as u128)
        .ok_or(DeepPoolError::MathOverflow)? as u64;

    require!(lp_amount > 0, DeepPoolError::ZeroDeposit);

    // 5. Mint LP tokens to provider
    let mint_key = ctx.accounts.pool.token_mint;
    let pool_seeds = &[
        POOL_SEED,
        mint_key.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.lp_mint.to_account_info(),
                to: ctx.accounts.provider_lp_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        lp_amount,
    )?;

    Ok(())
}
