use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    self, Mint as MintInterface, TokenAccount as TokenAccountInterface, TokenInterface,
    TransferChecked,
};

use crate::constants::*;
use crate::error::DeepPoolError;
use crate::events::SwapExecuted;
use crate::math;
use crate::state::Pool;

#[derive(AnchorDeserialize, AnchorSerialize)]
pub struct SwapArgs {
    pub amount_in: u64,
    /// Minimum acceptable output, in gross units (i.e. before any Token-2022
    /// transfer fee skimmed from the recipient side on the buy path). The
    /// actual amount the user wallet receives may be slightly less if the
    /// token has a transfer fee — wallets should subtract the mint's
    /// `TransferFeeConfig` rate from this value to render the expected
    /// receipt. Sell path is unaffected (output is SOL, no transfer fee).
    pub minimum_out: u64,
    /// true = SOL → Token (buy), false = Token → SOL (sell).
    pub buy: bool,
}

#[event_cpi]
#[derive(Accounts)]
#[instruction(args: SwapArgs)]
pub struct Swap<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    // SOL source on buy / SOL sink on sell. Equals `user` for wallet callers.
    // CPI callers pass a distinct system-owned PDA they sign for.
    #[account(mut)]
    pub sol_source: Signer<'info>,
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
    // User's token account — ATA enforced. Must exist; callers prepend
    // `createAssociatedTokenAccountIdempotent` (SPL ATA program) when needed.
    // Dropping `init_if_needed` here is structural: a program-owned `user`
    // (e.g. a CPI vault) cannot fund rent via system_program, so the init
    // branch was already broken on the cold path for CPI callers. Keeping the
    // ATA constraint preserves substitution safety.
    #[account(
        mut,
        associated_token::mint = token_mint,
        associated_token::authority = user,
        associated_token::token_program = token_program,
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccountInterface>,
    #[account(constraint = token_program.key() == TOKEN_2022_PROGRAM_ID @ DeepPoolError::NotToken2022)]
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

/// Metrics from a swap leg, used to populate the post-trade event.
struct SwapMetrics {
    amount_in_gross: u64,
    amount_in_net: u64,
    amount_out_gross: u64,
    amount_out_net: u64,
    fee: u64,
}

pub(crate) fn handler(mut ctx: Context<Swap>, args: SwapArgs) -> Result<()> {
    require!(args.amount_in > 0, DeepPoolError::ZeroInput);
    let pool_info = ctx.accounts.pool.to_account_info();
    let sol_reserve = Pool::sol_reserve(&pool_info)?;
    let token_reserve = ctx.accounts.token_vault.amount;
    require!(
        sol_reserve > 0 && token_reserve > 0,
        DeepPoolError::EmptyPool
    );

    let metrics = if args.buy {
        handle_buy(&mut ctx, &args, sol_reserve, token_reserve)?
    } else {
        handle_sell(&mut ctx, &args, sol_reserve, token_reserve)?
    };

    // Post-trade reserves — read live so the event reflects committed state.
    let sol_reserve_after = Pool::sol_reserve(&ctx.accounts.pool.to_account_info())?;
    ctx.accounts.token_vault.reload()?;
    let token_reserve_after = ctx.accounts.token_vault.amount;

    let pool_key = ctx.accounts.pool.key();
    let user_key = ctx.accounts.user.key();
    let sol_source_key = ctx.accounts.sol_source.key();

    emit_cpi!(SwapExecuted {
        pool: pool_key,
        user: user_key,
        sol_source: sol_source_key,
        buy: args.buy,
        amount_in_gross: metrics.amount_in_gross,
        amount_in_net: metrics.amount_in_net,
        amount_out_gross: metrics.amount_out_gross,
        amount_out_net: metrics.amount_out_net,
        fee: metrics.fee,
        sol_reserve_after,
        token_reserve_after,
    });

    Ok(())
}

/// SOL → Token. SOL has no transfer fee, so fees are computed on the gross
/// input. The slippage check is on the gross token output (pre-Token-2022
/// fee); the recipient-side fee is reported via `amount_out_net` in the
/// emitted event.
fn handle_buy(
    ctx: &mut Context<Swap>,
    args: &SwapArgs,
    sol_reserve: u64,
    token_reserve: u64,
) -> Result<SwapMetrics> {
    let pool_info = ctx.accounts.pool.to_account_info();
    let pool_fee = math::calc_swap_fee(args.amount_in).ok_or(DeepPoolError::MathOverflow)?;
    let effective_in = args
        .amount_in
        .checked_sub(pool_fee)
        .ok_or(DeepPoolError::MathOverflow)?;
    let tokens_out = math::calc_swap_output(effective_in, sol_reserve, token_reserve)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(
        tokens_out >= args.minimum_out,
        DeepPoolError::SlippageExceeded
    );
    require!(tokens_out < token_reserve, DeepPoolError::EmptyPool);

    let user_token_balance_before = ctx.accounts.user_token_account.amount;

    // SOL in. Works for wallets directly and for program-derived system-owned
    // PDAs via invoke_signed. The system program enforces that `from` actually
    // had the lamports — no caller can claim amount_in without providing it.
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.sol_source.to_account_info(),
                to: pool_info,
            },
        ),
        args.amount_in,
    )?;

    let config_key = ctx.accounts.pool.config;
    let mint_key = ctx.accounts.pool.token_mint;
    let pool_seeds = &[
        POOL_SEED,
        config_key.as_ref(),
        mint_key.as_ref(),
        &[ctx.accounts.pool.bump],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    // Tokens out from vault to user.
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

    // User-side delta exposes Token-2022 transfer-fee leakage.
    ctx.accounts.user_token_account.reload()?;
    let user_received = ctx
        .accounts
        .user_token_account
        .amount
        .checked_sub(user_token_balance_before)
        .ok_or(DeepPoolError::MathOverflow)?;

    Ok(SwapMetrics {
        amount_in_gross: args.amount_in,
        amount_in_net: args.amount_in,
        amount_out_gross: tokens_out,
        amount_out_net: user_received,
        fee: pool_fee,
    })
}

/// Token → SOL. Tokens have a transfer fee, so fees are computed on the net
/// amount that actually landed in the vault. SOL has no transfer fee, so
/// `amount_out_net == amount_out_gross` and `minimum_out` matches what the
/// SOL sink receives.
fn handle_sell(
    ctx: &mut Context<Swap>,
    args: &SwapArgs,
    sol_reserve: u64,
    token_reserve: u64,
) -> Result<SwapMetrics> {
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

    // 2. Apply pool fee on net received tokens
    let pool_fee = math::calc_swap_fee(net_received).ok_or(DeepPoolError::MathOverflow)?;
    let effective_in = net_received
        .checked_sub(pool_fee)
        .ok_or(DeepPoolError::MathOverflow)?;

    // 3. Constant-product output
    let sol_out = math::calc_swap_output(effective_in, token_reserve, sol_reserve)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(sol_out >= args.minimum_out, DeepPoolError::SlippageExceeded);
    require!(sol_out < sol_reserve, DeepPoolError::EmptyPool);

    // 4. Transfer SOL from pool PDA to sol_source. Direct lamport manipulation
    //    is required here — system_program::transfer can't move funds out of a
    //    program-owned account. The `sol_out < sol_reserve` check above
    //    already guarantees `lamports - sol_out > rent_exempt`; the explicit
    //    pre-mutation assertion below makes the invariant visible at the
    //    mutation site and survives any future refactor that weakens the
    //    upstream precondition.
    let pool_info = ctx.accounts.pool.to_account_info();
    let rent = Rent::get()?;
    require!(
        pool_info.lamports().saturating_sub(sol_out) >= rent.minimum_balance(Pool::LEN),
        DeepPoolError::MinimumLiquidityRequired
    );
    pool_info.sub_lamports(sol_out)?;
    ctx.accounts
        .sol_source
        .to_account_info()
        .add_lamports(sol_out)?;

    Ok(SwapMetrics {
        amount_in_gross: args.amount_in,
        amount_in_net: net_received,
        amount_out_gross: sol_out,
        amount_out_net: sol_out,
        fee: pool_fee,
    })
}
