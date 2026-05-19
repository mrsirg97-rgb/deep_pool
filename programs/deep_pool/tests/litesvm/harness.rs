// Litesvm test harness for deep_pool. Provides:
//   - Env::new bootstraps a fresh LiteSVM with deep_pool loaded.
//   - Mint helpers: mint_with_fee(bps) creates a Token-2022 mint with the
//     given transfer-fee config (0 = no fee). Returns the mint + an authority
//     keypair that can MintTo.
//   - Pool helpers: create_pool, add_liquidity, remove_liquidity, swap_buy,
//     swap_sell — wrap each instruction in a typed Result<()>.

#![allow(dead_code)]

use std::path::PathBuf;

use anchor_lang::{prelude::Pubkey, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_sdk::{
    account::ReadableAccount,
    hash::Hash,
    instruction::{Instruction, InstructionError},
    native_token::LAMPORTS_PER_SOL,
    signature::Keypair,
    signer::Signer,
    transaction::{Transaction, TransactionError},
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

use deep_pool::{
    constants::{LP_MINT_SEED, POOL_SEED, TOKEN_2022_PROGRAM_ID, VAULT_SEED},
    state::Pool,
};

// ============================================================================
// File paths
// ============================================================================

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn deep_pool_so() -> Vec<u8> {
    let path = workspace_root().join("target/deploy/deep_pool.so");
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "deep_pool.so missing at {:?}: {}. Run `cargo build-sbf --manifest-path programs/deep_pool/Cargo.toml` first.",
            path, e
        )
    })
}

// ============================================================================
// Env
// ============================================================================

pub struct Env {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

impl Env {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(deep_pool::ID, &deep_pool_so())
            .expect("add deep_pool program");
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1000 * LAMPORTS_PER_SOL)
            .unwrap();
        Env { svm, payer }
    }

    pub fn latest_blockhash(&self) -> Hash {
        self.svm.latest_blockhash()
    }

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<(), TransactionError> {
        let payer = signers
            .first()
            .expect("at least one signer (payer)")
            .pubkey();
        self.svm.expire_blockhash();
        let mut tx = Transaction::new_with_payer(ixs, Some(&payer));
        tx.sign(signers, self.latest_blockhash());
        match self.svm.send_transaction(tx) {
            Ok(_) => Ok(()),
            Err(failed) => {
                if std::env::var("LITESVM_LOGS").is_ok() {
                    eprintln!("--- tx failed: {:?} ---", failed.err);
                    for line in &failed.meta.logs {
                        eprintln!("{}", line);
                    }
                }
                Err(failed.err)
            }
        }
    }

    pub fn airdrop(&mut self, to: &Pubkey, lamports: u64) {
        self.svm.airdrop(to, lamports).unwrap();
    }

    pub fn new_funded(&mut self, lamports: u64) -> Keypair {
        let k = Keypair::new();
        self.airdrop(&k.pubkey(), lamports);
        k
    }

    pub fn balance(&self, pubkey: &Pubkey) -> u64 {
        self.svm
            .get_account(pubkey)
            .map(|a| a.lamports)
            .unwrap_or(0)
    }
}

// ============================================================================
// Token-2022 mint creation
// ============================================================================

/// Create a Token-2022 mint with the given transfer-fee bps (0 = no extension).
/// Returns the mint pubkey and a separate authority keypair that holds
/// mint/freeze/transfer-fee config authority — tests can MintTo with it.
pub fn create_mint(env: &mut Env, transfer_fee_bps: u16, decimals: u8) -> (Pubkey, Keypair) {
    use spl_token_2022::{
        extension::{transfer_fee, ExtensionType},
        instruction as token_ix,
        state::Mint,
    };

    let mint = Keypair::new();
    let authority = env.new_funded(LAMPORTS_PER_SOL);

    let extensions = if transfer_fee_bps > 0 {
        vec![ExtensionType::TransferFeeConfig]
    } else {
        vec![]
    };
    let space = ExtensionType::try_calculate_account_len::<Mint>(&extensions).unwrap();
    let rent = env.svm.minimum_balance_for_rent_exemption(space);

    let create = system_instruction::create_account(
        &env.payer.pubkey(),
        &mint.pubkey(),
        rent,
        space as u64,
        &TOKEN_2022_PROGRAM_ID,
    );

    let mut ixs = vec![create];
    if transfer_fee_bps > 0 {
        let init_fee = transfer_fee::instruction::initialize_transfer_fee_config(
            &TOKEN_2022_PROGRAM_ID,
            &mint.pubkey(),
            Some(&authority.pubkey()),
            Some(&authority.pubkey()),
            transfer_fee_bps,
            u64::MAX,
        )
        .unwrap();
        ixs.push(init_fee);
    }
    let init_mint = token_ix::initialize_mint2(
        &TOKEN_2022_PROGRAM_ID,
        &mint.pubkey(),
        &authority.pubkey(),
        Some(&authority.pubkey()),
        decimals,
    )
    .unwrap();
    ixs.push(init_mint);

    let payer = clone_keypair(&env.payer);
    env.send(&ixs, &[&payer, &mint])
        .expect("create_mint failed");
    (mint.pubkey(), authority)
}

/// Create the recipient's ATA for `mint` and mint `amount` raw tokens to it.
pub fn mint_to_user(
    env: &mut Env,
    mint: &Pubkey,
    mint_authority: &Keypair,
    recipient: &Keypair,
    amount: u64,
) -> Pubkey {
    use spl_token_2022::instruction as token_ix;

    let ata = derive_ata(&recipient.pubkey(), mint, &TOKEN_2022_PROGRAM_ID);
    let create_ata = build_create_ata_idempotent_ix(
        &env.payer.pubkey(),
        &recipient.pubkey(),
        mint,
        &TOKEN_2022_PROGRAM_ID,
    );
    let mint_to = token_ix::mint_to(
        &TOKEN_2022_PROGRAM_ID,
        mint,
        &ata,
        &mint_authority.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    let payer = clone_keypair(&env.payer);
    let authority = clone_keypair(mint_authority);
    env.send(&[create_ata, mint_to], &[&payer, &authority])
        .expect("mint_to_user failed");
    ata
}

// ============================================================================
// Pool PDA derivation
// ============================================================================

pub fn derive_pool(config: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[POOL_SEED, config.as_ref(), mint.as_ref()], &deep_pool::ID)
}

pub fn derive_vault(pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VAULT_SEED, pool.as_ref()], &deep_pool::ID)
}

pub fn derive_lp_mint(pool: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[LP_MINT_SEED, pool.as_ref()], &deep_pool::ID)
}

pub fn derive_event_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &deep_pool::ID).0
}

// ============================================================================
// PoolCtx — everything a test needs after create_pool
// ============================================================================

#[derive(Clone, Debug)]
pub struct PoolCtx {
    pub creator: Pubkey,
    pub config: Pubkey,
    pub mint: Pubkey,
    pub pool: Pubkey,
    pub token_vault: Pubkey,
    pub lp_mint: Pubkey,
}

// ============================================================================
// Read helpers
// ============================================================================

pub fn get_pool(env: &Env, pool: &Pubkey) -> Pool {
    let acct = env
        .svm
        .get_account(pool)
        .unwrap_or_else(|| panic!("pool {} missing", pool));
    let mut data = acct.data();
    use anchor_lang::AccountDeserialize;
    Pool::try_deserialize(&mut data).expect("Pool deserialize failed")
}

pub fn get_token_amount(env: &Env, ata: &Pubkey) -> u64 {
    let acct = env
        .svm
        .get_account(ata)
        .unwrap_or_else(|| panic!("token account {} missing", ata));
    u64::from_le_bytes(acct.data()[64..72].try_into().unwrap())
}

// ============================================================================
// Instruction wrappers
// ============================================================================

/// Create a pool. Creator is also the config signer (wallet-style caller).
/// Returns a PoolCtx with the derived addresses.
pub fn create_pool(
    env: &mut Env,
    creator: &Keypair,
    mint: &Pubkey,
    initial_token_amount: u64,
    initial_sol_amount: u64,
) -> Result<PoolCtx, TransactionError> {
    // ATA via local derive helper (no spl-ata dep)
    let config = clone_keypair(creator);
    let (pool, _) = derive_pool(&config.pubkey(), mint);
    let (token_vault, _) = derive_vault(&pool);
    let (lp_mint, _) = derive_lp_mint(&pool);
    let creator_token_account = derive_ata(&creator.pubkey(), mint, &TOKEN_2022_PROGRAM_ID);
    let creator_lp_account = derive_ata(&creator.pubkey(), &lp_mint, &TOKEN_2022_PROGRAM_ID);
    let pool_lp_account = derive_ata(&pool, &lp_mint, &TOKEN_2022_PROGRAM_ID);

    let ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::CreatePool {
            creator: creator.pubkey(),
            config: config.pubkey(),
            token_mint: *mint,
            pool,
            token_vault,
            lp_mint,
            creator_token_account,
            creator_lp_account,
            pool_lp_account,
            token_program: TOKEN_2022_PROGRAM_ID,
            associated_token_program: spl_ata_program_id(),
            system_program: system_program::ID,
            event_authority: derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::CreatePool {
            args: deep_pool::instructions::create_pool::CreatePoolArgs {
                initial_token_amount,
                initial_sol_amount,
            },
        }
        .data(),
    };
    env.send(&[ix], &[creator, &config])?;
    Ok(PoolCtx {
        creator: creator.pubkey(),
        config: config.pubkey(),
        mint: *mint,
        pool,
        token_vault,
        lp_mint,
    })
}

pub fn add_liquidity(
    env: &mut Env,
    provider: &Keypair,
    p: &PoolCtx,
    token_amount: u64,
    max_sol_amount: u64,
    min_lp_out: u64,
) -> Result<(), TransactionError> {
    // ATA via local derive helper (no spl-ata dep)
    let provider_token = derive_ata(&provider.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let provider_lp = derive_ata(&provider.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let pool_lp = derive_ata(&p.pool, &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::AddLiquidity {
            provider: provider.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.token_vault,
            lp_mint: p.lp_mint,
            provider_token_account: provider_token,
            provider_lp_account: provider_lp,
            pool_lp_account: pool_lp,
            token_program: TOKEN_2022_PROGRAM_ID,
            associated_token_program: spl_ata_program_id(),
            system_program: system_program::ID,
            event_authority: derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::AddLiquidity {
            args: deep_pool::instructions::add_liquidity::AddLiquidityArgs {
                token_amount,
                max_sol_amount,
                min_lp_out,
            },
        }
        .data(),
    };
    env.send(&[ix], &[provider])
}

pub fn remove_liquidity(
    env: &mut Env,
    provider: &Keypair,
    p: &PoolCtx,
    lp_amount: u64,
    min_sol_out: u64,
    min_tokens_out: u64,
) -> Result<(), TransactionError> {
    // ATA via local derive helper (no spl-ata dep)
    let provider_token = derive_ata(&provider.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let provider_lp = derive_ata(&provider.pubkey(), &p.lp_mint, &TOKEN_2022_PROGRAM_ID);
    let ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::RemoveLiquidity {
            provider: provider.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.token_vault,
            lp_mint: p.lp_mint,
            provider_token_account: provider_token,
            provider_lp_account: provider_lp,
            token_program: TOKEN_2022_PROGRAM_ID,
            associated_token_program: spl_ata_program_id(),
            system_program: system_program::ID,
            event_authority: derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::RemoveLiquidity {
            args: deep_pool::instructions::remove_liquidity::RemoveLiquidityArgs {
                lp_amount,
                min_sol_out,
                min_tokens_out,
            },
        }
        .data(),
    };
    env.send(&[ix], &[provider])
}

pub fn swap(
    env: &mut Env,
    user: &Keypair,
    p: &PoolCtx,
    amount_in: u64,
    minimum_out: u64,
    buy: bool,
) -> Result<(), TransactionError> {
    // ATA via local derive helper (no spl-ata dep)
    let user_token = derive_ata(&user.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::Swap {
            user: user.pubkey(),
            sol_source: user.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.token_vault,
            user_token_account: user_token,
            token_program: TOKEN_2022_PROGRAM_ID,
            associated_token_program: spl_ata_program_id(),
            system_program: system_program::ID,
            event_authority: derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::Swap {
            args: deep_pool::instructions::swap::SwapArgs {
                amount_in,
                minimum_out,
                buy,
            },
        }
        .data(),
    };
    env.send(&[ix], &[user])
}

// ============================================================================
// Error helpers
// ============================================================================

pub fn anchor_err_code(err: &TransactionError) -> Option<u32> {
    if let TransactionError::InstructionError(_, ix_err) = err {
        if let InstructionError::Custom(code) = ix_err {
            return Some(*code);
        }
    }
    None
}

#[macro_export]
macro_rules! expect_err {
    ($result:expr, $variant:expr) => {{
        let res = $result;
        let err = res.expect_err("expected error, got Ok");
        let code = $crate::harness::anchor_err_code(&err)
            .unwrap_or_else(|| panic!("expected Anchor Custom error, got: {:?}", err));
        let expected = ($variant as u32) + 6000;
        assert_eq!(
            code, expected,
            "expected error code {} ({:?}), got {}",
            expected, $variant, code
        );
    }};
}

// ============================================================================
// Internal
// ============================================================================

fn clone_keypair(k: &Keypair) -> Keypair {
    #[allow(deprecated)]
    Keypair::from_bytes(&k.to_bytes()).unwrap()
}

pub fn spl_ata_program_id() -> Pubkey {
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
        .parse()
        .unwrap()
}

/// Derive an ATA address generically (matches spl-associated-token-account's
/// `derive_ata` without pulling in the dep).
pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &spl_ata_program_id(),
    )
    .0
}

/// Build an ATA Program "CreateIdempotent" instruction (data tag = 1).
pub fn build_create_ata_idempotent_ix(
    payer: &Pubkey,
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    use solana_sdk::instruction::AccountMeta;
    let ata = derive_ata(wallet, mint, token_program);
    Instruction {
        program_id: spl_ata_program_id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(*wallet, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: vec![1],
    }
}
