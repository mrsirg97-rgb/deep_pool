use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        self, Mint as MintInterface, MintTo, TokenAccount as TokenAccountInterface, TokenInterface,
        TransferChecked,
    },
};

use crate::constants::*;
use crate::error::DeepPoolError;
use crate::events::LiquidityAdded;
use crate::math;
use crate::state::Pool;

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct AddLiquidityArgs {
    pub token_amount: u64,
    pub max_sol_amount: u64,
    pub min_lp_out: u64,
}

#[event_cpi]
#[derive(Accounts)]
#[instruction(args: AddLiquidityArgs)]
pub struct AddLiquidity<'info> {
    #[account(mut)]
    pub provider: Signer<'info>,
    #[account(
        mut,
        seeds = [POOL_SEED, pool.config.as_ref(), pool.token_mint.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Box<Account<'info, Pool>>,
    #[account(address = pool.token_mint)]
    pub token_mint: Box<InterfaceAccount<'info, MintInterface>>,
    #[account(mut, address = pool.token_vault)]
    pub token_vault: Box<InterfaceAccount<'info, TokenAccountInterface>>,
    #[account(mut, address = pool.lp_mint)]
    pub lp_mint: Box<InterfaceAccount<'info, MintInterface>>,
    #[account(
        mut,
        associated_token::mint = token_mint,
        associated_token::authority = provider,
        associated_token::token_program = token_program,
    )]
    pub provider_token_account: Box<InterfaceAccount<'info, TokenAccountInterface>>,
    #[account(
        init_if_needed,
        payer = provider,
        associated_token::mint = lp_mint,
        associated_token::authority = provider,
        associated_token::token_program = token_program,
    )]
    pub provider_lp_account: Box<InterfaceAccount<'info, TokenAccountInterface>>,
    // Pool PDA's LP account — receives 7.5% (permanently locked).
    #[account(
        mut,
        associated_token::mint = lp_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub pool_lp_account: Box<InterfaceAccount<'info, TokenAccountInterface>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<AddLiquidity>, args: AddLiquidityArgs) -> Result<()> {
    require!(args.token_amount > 0, DeepPoolError::ZeroDeposit);
    let pool_info = ctx.accounts.pool.to_account_info();
    let sol_reserve = Pool::sol_reserve(&pool_info)?;
    let token_reserve = ctx.accounts.token_vault.amount;
    require!(
        sol_reserve > 0 && token_reserve > 0,
        DeepPoolError::EmptyPool
    );
    let lp_supply = ctx.accounts.lp_mint.supply;
    require!(lp_supply > 0, DeepPoolError::EmptyPool);

    // 1. Compute required SOL for proportional deposit
    let sol_required = math::calc_lp_redeem(args.token_amount, sol_reserve, token_reserve)
        .ok_or(DeepPoolError::MathOverflow)?;
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
    let net_tokens = ctx
        .accounts
        .token_vault
        .amount
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
    let lp_amount = math::calc_lp_mint(lp_supply, net_tokens, token_reserve)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(lp_amount > 0, DeepPoolError::ZeroDeposit);
    require!(
        lp_amount >= args.min_lp_out,
        DeepPoolError::TokenOutputSlippage
    );

    // 5. Lock 7.5% of LP in the pool PDA — permanently inaccessible
    let lp_burn = (lp_amount as u128)
        .checked_mul(LP_LOCK_PROVIDER_BPS as u128)
        .ok_or(DeepPoolError::MathOverflow)?
        .checked_div(10000)
        .ok_or(DeepPoolError::MathOverflow)? as u64;
    let lp_to_provider = lp_amount
        .checked_sub(lp_burn)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(lp_to_provider > 0, DeepPoolError::ZeroDeposit);

    // 6. Mint LP: 92.5% to provider, 7.5% to pool PDA (permanently locked)
    let config_key = ctx.accounts.pool.config;
    let mint_key = ctx.accounts.pool.token_mint;
    let pool_seeds = &[
        POOL_SEED,
        config_key.as_ref(),
        mint_key.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    // 92.5% to provider
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
        lp_to_provider,
    )?;

    // 7.5% to pool PDA — locked forever
    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.lp_mint.to_account_info(),
                to: ctx.accounts.pool_lp_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        lp_burn,
    )?;

    // Post-state for the event. Vault was reloaded after the inbound transfer
    // and untouched since; lp_mint reloaded to capture post-mint supply.
    let sol_reserve_after = Pool::sol_reserve(&ctx.accounts.pool.to_account_info())?;
    let token_reserve_after = ctx.accounts.token_vault.amount;
    ctx.accounts.lp_mint.reload()?;
    let lp_supply_after = ctx.accounts.lp_mint.supply;
    let pool_key = ctx.accounts.pool.key();
    let provider_key = ctx.accounts.provider.key();

    emit_cpi!(LiquidityAdded {
        pool: pool_key,
        provider: provider_key,
        sol_in_gross: sol_required,
        sol_in_net: sol_required,
        tokens_in_gross: args.token_amount,
        tokens_in_net: net_tokens,
        lp_to_provider,
        lp_locked: lp_burn,
        sol_reserve_after,
        token_reserve_after,
        lp_supply_after,
    });

    Ok(())
}
