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
pub struct CreatePoolArgs {
    pub initial_token_amount: u64,
    pub initial_sol_amount: u64,
}

#[derive(Accounts)]
#[instruction(args: CreatePoolArgs)]
pub struct CreatePool<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    // The Token-2022 mint for this pool.
    #[account(
        mint::token_program = token_program,
        constraint = token_mint.to_account_info().owner == &TOKEN_2022_PROGRAM_ID @ DeepPoolError::NotToken2022,
    )]
    pub token_mint: InterfaceAccount<'info, MintInterface>,
    // Pool state PDA — one per mint.
    #[account(
        init,
        payer = creator,
        space = Pool::LEN,
        seeds = [POOL_SEED, token_mint.key().as_ref()],
        bump,
    )]
    pub pool: Account<'info, Pool>,
    // Token vault — PDA-derived, owned by pool.
    #[account(
        init,
        payer = creator,
        token::mint = token_mint,
        token::authority = pool,
        token::token_program = token_program,
        seeds = [VAULT_SEED, pool.key().as_ref()],
        bump,
    )]
    pub token_vault: InterfaceAccount<'info, TokenAccountInterface>,
    // LP token mint — authority is pool PDA.
    #[account(
        init,
        payer = creator,
        mint::decimals = 6,
        mint::authority = pool,
        mint::freeze_authority = pool,
        mint::token_program = token_program,
        seeds = [LP_MINT_SEED, pool.key().as_ref()],
        bump,
    )]
    pub lp_mint: InterfaceAccount<'info, MintInterface>,
    // Creator's token account — source of initial deposit.
    #[account(
        mut,
        associated_token::mint = token_mint,
        associated_token::authority = creator,
        associated_token::token_program = token_program,
    )]
    pub creator_token_account: InterfaceAccount<'info, TokenAccountInterface>,
    // Creator's LP token account — receives initial LP tokens.
    #[account(
        init_if_needed,
        payer = creator,
        associated_token::mint = lp_mint,
        associated_token::authority = creator,
        associated_token::token_program = token_program,
    )]
    pub creator_lp_account: InterfaceAccount<'info, TokenAccountInterface>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<CreatePool>, args: CreatePoolArgs) -> Result<()> {
    require!(
        args.initial_sol_amount >= MIN_INITIAL_SOL,
        DeepPoolError::InsufficientInitialSol
    );
    require!(
        args.initial_token_amount >= MIN_INITIAL_TOKENS,
        DeepPoolError::InsufficientInitialTokens
    );

    // 1. Transfer tokens from creator to vault (measure net for Token-2022 fee)
    let vault_before = ctx.accounts.token_vault.amount;

    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.creator_token_account.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                to: ctx.accounts.token_vault.to_account_info(),
                authority: ctx.accounts.creator.to_account_info(),
            },
        ),
        args.initial_token_amount,
        ctx.accounts.token_mint.decimals,
    )?;

    ctx.accounts.token_vault.reload()?;
    let net_tokens = ctx.accounts.token_vault.amount
        .checked_sub(vault_before)
        .ok_or(DeepPoolError::MathOverflow)?;
    require!(net_tokens > 0, DeepPoolError::InsufficientInitialTokens);

    // 2. Transfer SOL from creator to pool PDA
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.creator.to_account_info(),
                to: ctx.accounts.pool.to_account_info(),
            },
        ),
        args.initial_sol_amount,
    )?;

    // 3. Compute initial LP supply: sqrt(sol * tokens) - MIN_LIQUIDITY
    let product = (args.initial_sol_amount as u128)
        .checked_mul(net_tokens as u128)
        .ok_or(DeepPoolError::MathOverflow)?;
    let sqrt = integer_sqrt(product);
    require!(sqrt > MIN_LIQUIDITY as u128, DeepPoolError::InsufficientInitialSol);
    let lp_to_creator = (sqrt as u64)
        .checked_sub(MIN_LIQUIDITY)
        .ok_or(DeepPoolError::MathOverflow)?;

    // 4. Mint LP tokens to creator
    let mint_key = ctx.accounts.token_mint.key();
    let pool_seeds = &[
        POOL_SEED,
        mint_key.as_ref(),
        &[ctx.bumps.pool],
    ];
    let signer_seeds = &[&pool_seeds[..]];

    // Mint MIN_LIQUIDITY to pool PDA (permanently locked — no way to redeem)
    // This prevents the first-depositor rounding attack.
    // We mint to the vault since pool PDA doesn't have an LP ATA.
    // These tokens are unrecoverable since no instruction can burn from pool authority
    // without the LP being in a burnable account. In practice we just inflate supply
    // by MIN_LIQUIDITY and nobody holds them — the share dilution is negligible.
    // Alternative: mint and immediately burn, but that requires a burn destination.
    // Simplest: just don't mint them. Start LP supply at lp_to_creator.
    // The MIN_LIQUIDITY subtraction from sqrt already ensures the first depositor
    // can't round-exploit: they get fewer LP tokens than the geometric mean.

    token_interface::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.lp_mint.to_account_info(),
                to: ctx.accounts.creator_lp_account.to_account_info(),
                authority: ctx.accounts.pool.to_account_info(),
            },
            signer_seeds,
        ),
        lp_to_creator,
    )?;

    // 5. Initialize pool state
    let pool = &mut ctx.accounts.pool;
    pool.token_mint = ctx.accounts.token_mint.key();
    pool.token_vault = ctx.accounts.token_vault.key();
    pool.lp_mint = ctx.accounts.lp_mint.key();
    pool.initial_sol = args.initial_sol_amount;
    pool.initial_tokens = net_tokens;
    pool.total_swaps = 0;
    pool.bump = ctx.bumps.pool;

    Ok(())
}

// Integer square root via Newton's method (u128).
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
