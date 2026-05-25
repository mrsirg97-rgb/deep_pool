//! Jupiter `Amm` trait implementation for DeepPool.
//!
//! Wraps the on-chain `deep_pool` program for use by Jupiter's router. The
//! quote math is `deep_pool::math` re-imported directly — there is one
//! source of truth and the off-chain quote cannot drift from the on-chain
//! swap outcome.
//!
//! Token-2022 transfer fees on the *token* leg are folded into quotes so
//! `Quote::out_amount` is what the user's wallet actually receives, not the
//! pre-transfer-fee gross. The SOL leg is native and has no transfer fee.
//!
//! Single direction per call: ExactIn only. ExactOut is feasible (the math
//! is invertible) but not required for initial Jupiter listing.
//!
//! ## Type bridging
//!
//! Three versions of `solana-pubkey` are present in the dependency graph
//! (2.x via anchor-lang 0.32, 3.x via jupiter-amm-interface 1.0.0-beta,
//! 4.x elsewhere). Rust treats each as a distinct type despite identical
//! byte layouts. Helpers `to_jup` / `to_anchor` bridge between them by
//! round-tripping through `to_bytes()`. All trait-facing surface uses the
//! jupiter type; conversions happen only at the anchor boundary.

use anchor_lang::{prelude::Pubkey as AnchorPubkey, AccountDeserialize, InstructionData};
use anyhow::{anyhow, Context as AnyhowContext, Result};
use jupiter_amm_interface::{
    try_get_account_data, AccountMap, Amm, AmmContext, KeyedAccount, Quote, QuoteParams, Swap,
    SwapAndAccountMetas, SwapMode, SwapParams,
};
use solana_instruction::AccountMeta;
use solana_pubkey::{pubkey, Pubkey};
use solana_rent::Rent;
use spl_token_2022::{
    extension::{
        transfer_fee::TransferFeeConfig, BaseStateWithExtensions, StateWithExtensions,
    },
    state::{Account as SplTokenAccount, Mint as SplMint},
};

use deep_pool::{
    constants::TOKEN_2022_PROGRAM_ID as ANCHOR_TOKEN_2022_PROGRAM_ID,
    math::{calc_swap_fee, calc_swap_output},
    state::Pool,
};

/// Wrapped SOL mint — Jupiter's convention for native SOL on either reserve
/// leg. The router transparently wraps/unwraps WSOL at the route boundary;
/// DeepPool itself only touches native SOL lamports.
const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

/// System program ID under the jup-pubkey type. Hard-coded to avoid a
/// version skew with `solana_system_interface::program::ID`.
const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

/// Token-2022 program ID under the jup-pubkey type.
const TOKEN_2022_PROGRAM_ID: Pubkey =
    pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// SPL Associated Token Account program ID under the jup-pubkey type.
const SPL_ATA_PROGRAM_ID: Pubkey =
    pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Bridge an `anchor_lang::prelude::Pubkey` into a `solana_pubkey::Pubkey`.
/// Round-trips through `to_bytes()` to defeat Rust's nominal typing across
/// duplicate-version dep graphs. See module-level "Type bridging" note.
fn to_jup(p: &AnchorPubkey) -> Pubkey {
    Pubkey::new_from_array(p.to_bytes())
}

/// Sanity check at build time: TOKEN_2022_PROGRAM_ID under the two type
/// universes must agree byte-for-byte. If anchor's constant ever drifts,
/// this becomes a const eval mismatch caught at compile time rather than
/// a silent runtime account-validation failure.
#[allow(dead_code)]
const _: () = {
    let anchor_bytes = ANCHOR_TOKEN_2022_PROGRAM_ID.to_bytes();
    let jup_bytes = TOKEN_2022_PROGRAM_ID.to_bytes();
    let mut i = 0;
    while i < 32 {
        assert!(
            anchor_bytes[i] == jup_bytes[i],
            "TOKEN_2022_PROGRAM_ID byte mismatch between anchor and jup type"
        );
        i += 1;
    }
};

/// Pre-allocated `Rent` sysvar default — DeepPool's `Pool` account size is
/// known at compile time, so the rent-exempt minimum can be computed without
/// a sysvar fetch (matches what the on-chain handler does via `Rent::get()`).
fn pool_rent_exempt() -> u64 {
    Rent::default().minimum_balance(Pool::LEN)
}

/// SPL ATA derivation — local copy to avoid the spl-associated-token-account
/// dep dragging in yet another solana-* version permutation.
fn derive_ata(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    let (ata, _) = Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &SPL_ATA_PROGRAM_ID,
    );
    ata
}

/// Anchor `emit_cpi!` event-authority PDA — `[b"__event_authority"]` under
/// the program ID. Required by every state-changing ix because every handler
/// is `#[event_cpi]`-decorated.
fn derive_event_authority(program_id: &Pubkey) -> Pubkey {
    let (pda, _) = Pubkey::find_program_address(&[b"__event_authority"], program_id);
    pda
}

/// Off-chain state for a single DeepPool pool, kept in sync with on-chain
/// state via `update()`. All pubkeys cached in the jupiter type so the
/// hot path (`quote`, `get_swap_and_account_metas`) does zero conversions.
#[derive(Clone)]
pub struct DeepPoolAmm {
    program_id: Pubkey,
    pool_key: Pubkey,
    token_mint: Pubkey,
    token_vault: Pubkey,
    /// Live SOL reserve, refreshed each `update`. Equals `pool.lamports() - rent_exempt`.
    sol_reserve: u64,
    /// Live token reserve, refreshed each `update`. Equals `vault.amount`.
    token_reserve: u64,
    /// Token-2022 transfer-fee config at the time of last `update`. Zero
    /// when the mint has no `TransferFeeConfig` extension or the active fee
    /// for the current epoch is zero.
    transfer_fee_bps: u16,
    /// Per-transfer maximum fee from the mint's `TransferFeeConfig` (u64::MAX
    /// when no cap). Required because Token-2022 caps the absolute fee a
    /// single transfer can take regardless of the bps rate.
    transfer_fee_maximum: u64,
}

impl DeepPoolAmm {
    /// Test-only constructor that bypasses on-chain account fetching. Used
    /// by the `quote_matches_math` test suite to verify the math wrapper
    /// without paying the cost of constructing real Token-2022 packed mint
    /// / vault account data (which would drag the spl-token-2022 pack API
    /// across the solana-version skew described in the module header).
    ///
    /// Not part of the public API. Callers that need a real `DeepPoolAmm`
    /// should go through `from_keyed_account` + `update`.
    #[doc(hidden)]
    pub fn new_for_test(
        program_id: Pubkey,
        pool_key: Pubkey,
        token_mint: Pubkey,
        token_vault: Pubkey,
        sol_reserve: u64,
        token_reserve: u64,
        transfer_fee_bps: u16,
        transfer_fee_maximum: u64,
    ) -> Self {
        Self {
            program_id,
            pool_key,
            token_mint,
            token_vault,
            sol_reserve,
            token_reserve,
            transfer_fee_bps,
            transfer_fee_maximum,
        }
    }

    /// Apply the Token-2022 transfer fee to an amount, returning what would
    /// actually land in the recipient account. Mirrors what the on-chain
    /// `transfer_checked` CPI yields — including **ceiling** division for
    /// the fee, matching `spl_token_2022::extension::transfer_fee::TransferFeeConfig::calculate_fee`.
    /// Floor division would under-charge by 1 lamport whenever the
    /// `amount * bps` product has a remainder, putting `quote.out_amount`
    /// off by 1 from the on-chain receipt.
    fn apply_transfer_fee(&self, amount: u64) -> u64 {
        if self.transfer_fee_bps == 0 || amount == 0 {
            return amount;
        }
        let numerator = (amount as u128).saturating_mul(self.transfer_fee_bps as u128);
        // Ceiling: (numerator + denom - 1) / denom
        let raw = numerator.saturating_add(9_999u128) / 10_000u128;
        let fee = u64::try_from(raw)
            .unwrap_or(u64::MAX)
            .min(self.transfer_fee_maximum);
        amount.saturating_sub(fee)
    }
}

impl Amm for DeepPoolAmm {
    fn from_keyed_account(keyed: &KeyedAccount, _ctx: &AmmContext) -> Result<Self> {
        let pool = Pool::try_deserialize(&mut keyed.account.data.as_slice())
            .context("deserialize Pool account")?;
        Ok(Self {
            // KeyedAccount fields already use the jupiter pubkey type, so
            // these direct assignments don't need conversion.
            program_id: keyed.account.owner,
            pool_key: keyed.key,
            token_mint: to_jup(&pool.token_mint),
            token_vault: to_jup(&pool.token_vault),
            // Reserves and fee are unknown until `update`; default to 0 so
            // `is_active` correctly reports false on a fresh, unupdated AMM.
            sol_reserve: 0,
            token_reserve: 0,
            transfer_fee_bps: 0,
            transfer_fee_maximum: u64::MAX,
        })
    }

    fn label(&self) -> String {
        "DeepPool".to_string()
    }

    fn program_id(&self) -> Pubkey {
        self.program_id
    }

    fn key(&self) -> Pubkey {
        self.pool_key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![WSOL_MINT, self.token_mint]
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        // pool — lamports change on every swap (sol_reserve = lamports - rent_exempt)
        // token_vault — amount changes on every swap
        // token_mint — TransferFeeConfig active rate is per-epoch; refresh on every cycle
        vec![self.pool_key, self.token_vault, self.token_mint]
    }

    fn update(&mut self, account_map: &AccountMap) -> Result<()> {
        // Pool — use the raw lamports off the Account; reserve = lamports - rent.
        let pool_account = account_map
            .get(&self.pool_key)
            .with_context(|| format!("missing pool account {}", self.pool_key))?;
        self.sol_reserve = pool_account.lamports.saturating_sub(pool_rent_exempt());

        // Vault — token amount from the SPL Token-2022 account.
        let vault_data = try_get_account_data(account_map, &self.token_vault)?;
        let vault_state = StateWithExtensions::<SplTokenAccount>::unpack(vault_data)
            .map_err(|e| anyhow!("unpack token vault: {e}"))?;
        self.token_reserve = vault_state.base.amount;

        // Mint — pull active transfer-fee rate + cap if the extension is present.
        let mint_data = try_get_account_data(account_map, &self.token_mint)?;
        let mint_state = StateWithExtensions::<SplMint>::unpack(mint_data)
            .map_err(|e| anyhow!("unpack mint: {e}"))?;
        match mint_state.get_extension::<TransferFeeConfig>() {
            Ok(cfg) => {
                // `newer_transfer_fee` is the active fee from
                // `epoch >= cfg.newer_transfer_fee.epoch`. Without a `Clock`
                // here, we use it unconditionally — correct in steady state.
                // For the rare in-flight epoch transition, the on-chain
                // transfer measurement still truth-corrects via the
                // `amount_out_net` event field; quote drift is bounded.
                let active = cfg.newer_transfer_fee;
                self.transfer_fee_bps = u16::from(active.transfer_fee_basis_points);
                self.transfer_fee_maximum = u64::from(active.maximum_fee);
            }
            Err(_) => {
                self.transfer_fee_bps = 0;
                self.transfer_fee_maximum = u64::MAX;
            }
        }

        Ok(())
    }

    fn quote(&self, params: &QuoteParams) -> Result<Quote> {
        if !matches!(params.swap_mode, SwapMode::ExactIn) {
            return Err(anyhow!("DeepPool only supports ExactIn quotes"));
        }
        if self.sol_reserve == 0 || self.token_reserve == 0 {
            return Err(anyhow!("pool is empty or not yet updated"));
        }

        let buy = if params.input_mint == WSOL_MINT && params.output_mint == self.token_mint {
            true
        } else if params.input_mint == self.token_mint && params.output_mint == WSOL_MINT {
            false
        } else {
            return Err(anyhow!(
                "unsupported mint pair: in={} out={} (pool trades WSOL/{})",
                params.input_mint,
                params.output_mint,
                self.token_mint
            ));
        };

        let amount = params.amount;
        if amount == 0 {
            return Err(anyhow!("zero input amount"));
        }

        let (in_amount, out_amount, fee_amount, fee_mint) = if buy {
            // SOL → Token. Fee is on the SOL input; output is gross tokens
            // out of the vault, then the user-side Token-2022 transfer fee
            // is skimmed from the recipient leg.
            let pool_fee = calc_swap_fee(amount).context("calc_swap_fee overflow")?;
            let effective_in = amount
                .checked_sub(pool_fee)
                .context("amount - pool_fee underflow")?;
            let tokens_out_gross =
                calc_swap_output(effective_in, self.sol_reserve, self.token_reserve)
                    .context("calc_swap_output overflow")?;
            if tokens_out_gross >= self.token_reserve {
                return Err(anyhow!("swap would drain vault"));
            }
            let tokens_out_net = self.apply_transfer_fee(tokens_out_gross);
            (amount, tokens_out_net, pool_fee, WSOL_MINT)
        } else {
            // Token → SOL. Token transfer fee skims on the way into the
            // vault, pool fee on the post-fee net, then constant-product.
            let net_in = self.apply_transfer_fee(amount);
            if net_in == 0 {
                return Err(anyhow!("transfer fee consumed entire input"));
            }
            let pool_fee = calc_swap_fee(net_in).context("calc_swap_fee overflow")?;
            let effective_in = net_in
                .checked_sub(pool_fee)
                .context("net_in - pool_fee underflow")?;
            let sol_out = calc_swap_output(effective_in, self.token_reserve, self.sol_reserve)
                .context("calc_swap_output overflow")?;
            if sol_out >= self.sol_reserve {
                return Err(anyhow!("swap would drain pool SOL"));
            }
            (amount, sol_out, pool_fee, self.token_mint)
        };

        // 0.25% — Token-2022 transfer fees are folded into out_amount above.
        // fee_pct stays a flat 0.0025; Jupiter uses it for display and
        // routing tiebreaks, not as a quote correction.
        let fee_pct = rust_decimal::Decimal::new(25, 4); // 0.0025

        Ok(Quote {
            in_amount,
            out_amount,
            fee_amount,
            fee_mint,
            fee_pct,
        })
    }

    fn get_swap_and_account_metas(&self, params: &SwapParams) -> Result<SwapAndAccountMetas> {
        // Direction check — Jupiter normalizes SOL → WSOL_MINT before this call.
        let _buy = if params.source_mint == WSOL_MINT && params.destination_mint == self.token_mint
        {
            true
        } else if params.source_mint == self.token_mint && params.destination_mint == WSOL_MINT {
            false
        } else {
            return Err(anyhow!(
                "unsupported swap direction: source={} destination={}",
                params.source_mint,
                params.destination_mint
            ));
        };

        // Wallet-style swap: user is both the token authority and the SOL
        // source/sink. Jupiter routes through user-owned ATAs.
        let user_token_account = derive_ata(&params.user, &self.token_mint, &TOKEN_2022_PROGRAM_ID);
        let event_authority = derive_event_authority(&self.program_id);

        // Build account metas in the exact order of `deep_pool::accounts::Swap`.
        // Manual construction (rather than `accounts::Swap { ... }.to_account_metas`)
        // avoids transiting anchor's `AccountMeta` type, which is from a
        // different solana-instruction version than jupiter expects.
        let account_metas = vec![
            // user — Signer, mut (pays SOL on buy, pays fees in either dir)
            AccountMeta::new(params.user, true),
            // sol_source — Signer, mut. Equal to user for wallet flows.
            AccountMeta::new(params.user, true),
            // pool — mut (lamports change)
            AccountMeta::new(self.pool_key, false),
            // token_mint — read-only
            AccountMeta::new_readonly(self.token_mint, false),
            // token_vault — mut (vault.amount changes)
            AccountMeta::new(self.token_vault, false),
            // user_token_account — mut
            AccountMeta::new(user_token_account, false),
            // token_program — read-only (Token-2022)
            AccountMeta::new_readonly(TOKEN_2022_PROGRAM_ID, false),
            // system_program — read-only
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            // event_authority — read-only (Anchor #[event_cpi] sink)
            AccountMeta::new_readonly(event_authority, false),
            // program — read-only (Anchor #[event_cpi] self-CPI target)
            AccountMeta::new_readonly(self.program_id, false),
        ];

        // NOTE: `Swap::TokenSwap` is a placeholder until `jupiter-amm-interface`
        // adds a `Swap::DeepPool` variant. The discriminant is used by
        // Jupiter's router for analytics / labeling, not for any on-chain
        // dispatch — the account metas + ix data alone determine the actual
        // CPI shape, and those are correct. Replace when the variant lands
        // upstream (typically via the same PR that adds DeepPool to the
        // AMM allowlist).
        Ok(SwapAndAccountMetas {
            swap: Swap::TokenSwap,
            account_metas,
        })
    }

    fn clone_amm(&self) -> Box<dyn Amm + Send + Sync> {
        Box::new(self.clone())
    }

    fn is_active(&self) -> bool {
        // A pool with both reserves > 0 can be quoted against. Pre-update
        // (reserves still at the zero defaults from `from_keyed_account`)
        // returns false, telling Jupiter to skip it until `update` runs.
        self.sol_reserve > 0 && self.token_reserve > 0
    }

    fn get_accounts_len(&self) -> usize {
        // Matches the metas vec built in `get_swap_and_account_metas`.
        10
    }
}

/// Encode the swap ix data — a thin pass-through so callers can build the
/// full `solana_instruction::Instruction` without depending on `deep_pool`
/// directly. Anchor's `InstructionData` already returns `Vec<u8>`, so no
/// cross-version type bridging is needed here.
pub fn swap_ix_data(amount_in: u64, minimum_out: u64, buy: bool) -> Vec<u8> {
    deep_pool::instruction::Swap {
        args: deep_pool::instructions::swap::SwapArgs {
            amount_in,
            minimum_out,
            buy,
        },
    }
    .data()
}
