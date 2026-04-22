use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{
        self, Mint as MintInterface, TokenAccount as TokenAccountInterface, TokenInterface,
        TransferChecked,
    },
};

use crate::constants::*;
use crate::error::DeepPoolError;
use crate::math;
use crate::state::Pool;

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct SwapArgs {
    pub amount_in: u64,
    pub minimum_out: u64,
    // true = SOL to Token (buy), false = Token to SOL (sell)
    pub buy: bool,
}

#[derive(Accounts)]
#[instruction(args: SwapArgs)]
pub struct Swap<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
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
    // User's token account — ATA enforced.
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = token_mint,
        associated_token::authority = user,
        associated_token::token_program = token_program,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccountInterface>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<Swap>, args: SwapArgs) -> Result<()> {
    require!(args.amount_in > 0, DeepPoolError::ZeroInput);
    let pool_info = ctx.accounts.pool.to_account_info();
    let sol_reserve = Pool::sol_reserve(&pool_info)?;
    let token_reserve = ctx.accounts.token_vault.amount;
    require!(
        sol_reserve > 0 && token_reserve > 0,
        DeepPoolError::EmptyPool
    );

    let config_key = ctx.accounts.pool.config;
    let mint_key = ctx.accounts.pool.token_mint;
    let pool_seeds = &[
        POOL_SEED,
        config_key.as_ref(),
        mint_key.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    if args.buy {
        // SOL → Token
        // CPI callers (e.g. torch_market) pre-deposit SOL before invoking swap,
        // so sol_reserve already includes amount_in. Wallet callers transfer via
        // System Program below, so sol_reserve is the true pre-swap reserve.
        let is_wallet = ctx.accounts.user.owner == &anchor_lang::system_program::ID;
        let base_sol_reserve = if is_wallet {
            sol_reserve
        } else {
            sol_reserve
                .checked_sub(args.amount_in)
                .ok_or(DeepPoolError::InsufficientDeposit)?
        };

        // 1. Apply fee on input SOL
        let fee = math::calc_swap_fee(args.amount_in).ok_or(DeepPoolError::MathOverflow)?;
        let effective_in = args
            .amount_in
            .checked_sub(fee)
            .ok_or(DeepPoolError::MathOverflow)?;

        // 2. Constant product output
        let tokens_out = math::calc_swap_output(effective_in, base_sol_reserve, token_reserve)
            .ok_or(DeepPoolError::MathOverflow)?;
        require!(
            tokens_out >= args.minimum_out,
            DeepPoolError::SlippageExceeded
        );
        require!(tokens_out < token_reserve, DeepPoolError::EmptyPool);

        // 3. Transfer SOL from wallet callers via System Program
        if is_wallet {
            anchor_lang::system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.user.to_account_info(),
                        to: pool_info,
                    },
                ),
                args.amount_in,
            )?;
        }

        // 4. Transfer tokens from vault to user
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.token_vault.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                },
                signer_seeds,
            ),
            tokens_out,
            ctx.accounts.token_mint.decimals,
        )?;
    } else {
        // Token → SOL
        // 1. Transfer tokens from user to vault (measure net received)
        let vault_before = ctx.accounts.token_vault.amount;

        token_interface::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.user_token_account.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.token_vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            args.amount_in,
            ctx.accounts.token_mint.decimals,
        )?;

        ctx.accounts.token_vault.reload()?;
        let net_received = ctx
            .accounts
            .token_vault
            .amount
            .checked_sub(vault_before)
            .ok_or(DeepPoolError::MathOverflow)?;
        require!(net_received > 0, DeepPoolError::ZeroInput);

        // 2. Apply fee on net received tokens
        let fee = math::calc_swap_fee(net_received).ok_or(DeepPoolError::MathOverflow)?;
        let effective_in = net_received
            .checked_sub(fee)
            .ok_or(DeepPoolError::MathOverflow)?;

        // 3. Constant product output
        let sol_out = math::calc_swap_output(effective_in, token_reserve, sol_reserve)
            .ok_or(DeepPoolError::MathOverflow)?;
        require!(sol_out >= args.minimum_out, DeepPoolError::SlippageExceeded);
        require!(sol_out < sol_reserve, DeepPoolError::EmptyPool);

        // 4. Transfer SOL from pool PDA to user
        let pool_account = ctx.accounts.pool.to_account_info();
        **pool_account.try_borrow_mut_lamports()? = pool_account
            .lamports()
            .checked_sub(sol_out)
            .ok_or(DeepPoolError::MathOverflow)?;
        let user_account = ctx.accounts.user.to_account_info();
        **user_account.try_borrow_mut_lamports()? = user_account
            .lamports()
            .checked_add(sol_out)
            .ok_or(DeepPoolError::MathOverflow)?;
    }

    // Update swap counter
    let pool = &mut ctx.accounts.pool;
    pool.total_swaps = pool.total_swaps.saturating_add(1);

    Ok(())
}
