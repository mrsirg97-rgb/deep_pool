//! End-to-end roundtrip tests: `DeepPoolAmm::quote()` must match the actual
//! on-chain swap output to the lamport.
//!
//! This is the canonical correctness guarantee for the Jupiter integration.
//! Mock tests in `quote_matches_math.rs` prove the math wrapper matches
//! `deep_pool::math`; *these* tests prove the wrapper matches what a real
//! swap on a real Solana VM produces, including:
//!
//!   - constant-product math at the on-chain reserves
//!   - the 25 bps pool fee at the right point in the pipeline
//!   - Token-2022 transfer-fee skimming on the right leg
//!   - account/mint/vault layout assumptions
//!
//! Each scenario:
//!   1. Boot litesvm, load `deep_pool.so`
//!   2. Create a Token-2022 mint (with or without transfer fee)
//!   3. Seed a pool via the real `create_pool` ix
//!   4. Pull the on-chain pool/vault/mint accounts out of SVM state
//!   5. Build a `DeepPoolAmm` from those accounts; `update()`
//!   6. Call `quote()` — record predicted out_amount
//!   7. Execute the actual swap ix on the SVM
//!   8. Measure the actual out amount from balance deltas
//!   9. `assert_eq!(quote.out_amount, actual)`
//!
//! ## Type bridging
//!
//! Litesvm pulls solana-sdk 2.x (Pubkey 2.4 / Account 2.x / Instruction 2.x);
//! `jupiter-amm-interface` uses solana-pubkey 3.x / solana-account 3.x /
//! solana-instruction 3.x. The two universes don't unify at the type level.
//! Helpers `sdk_to_jup` / `sdk_account_to_jup` bridge by round-tripping
//! through `to_bytes()`. Same pattern as the lib code, applied to test
//! scope.

use anchor_lang::{InstructionData, ToAccountMetas};
use jupiter_amm_interface::{
    AccountMap, Amm, AmmContext, KeyedAccount, QuoteParams, SwapMode,
};
use litesvm::LiteSVM;
use solana_pubkey::{pubkey, Pubkey as JupPubkey};
use solana_sdk::{
    account::ReadableAccount,
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey as SdkPubkey,
    signature::Keypair,
    signer::Signer,
    transaction::{Transaction, TransactionError},
};
use solana_system_interface::{instruction as system_instruction, program as system_program};
use std::path::PathBuf;

use deep_pool::constants::{LP_MINT_SEED, POOL_SEED, TOKEN_2022_PROGRAM_ID, VAULT_SEED};
use jupiter_amm_deeppool::DeepPoolAmm;

const WSOL_MINT: JupPubkey = pubkey!("So11111111111111111111111111111111111111112");

// ============================================================================
// Pubkey / Account bridging (sdk 2.x ↔ jup 3.x)
// ============================================================================

fn sdk_to_jup(p: &SdkPubkey) -> JupPubkey {
    JupPubkey::new_from_array(p.to_bytes())
}

fn sdk_account_to_jup(acc: &solana_sdk::account::Account) -> solana_account::Account {
    solana_account::Account {
        lamports: acc.lamports,
        data: acc.data.clone(),
        owner: sdk_to_jup(&acc.owner),
        executable: acc.executable,
        rent_epoch: acc.rent_epoch,
    }
}

// ============================================================================
// Litesvm env
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

struct Env {
    svm: LiteSVM,
    payer: Keypair,
}

impl Env {
    fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(deep_pool::ID, &deep_pool_so()).unwrap();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1000 * LAMPORTS_PER_SOL).unwrap();
        Env { svm, payer }
    }

    fn latest_blockhash(&self) -> Hash {
        self.svm.latest_blockhash()
    }

    fn send(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<(), TransactionError> {
        let payer = signers.first().unwrap().pubkey();
        self.svm.expire_blockhash();
        let mut tx = Transaction::new_with_payer(ixs, Some(&payer));
        tx.sign(signers, self.latest_blockhash());
        self.svm.send_transaction(tx).map(|_| ()).map_err(|f| f.err)
    }

    fn new_funded(&mut self, lamports: u64) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), lamports).unwrap();
        k
    }

    fn balance(&self, pubkey: &SdkPubkey) -> u64 {
        self.svm.get_account(pubkey).map(|a| a.lamports).unwrap_or(0)
    }

    fn get_account(&self, pubkey: &SdkPubkey) -> solana_sdk::account::Account {
        self.svm
            .get_account(pubkey)
            .unwrap_or_else(|| panic!("account {} missing", pubkey))
    }
}

// ============================================================================
// SPL ATA helpers (no spl-ata dep)
// ============================================================================

fn spl_ata_program_id() -> SdkPubkey {
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".parse().unwrap()
}

fn derive_ata(wallet: &SdkPubkey, mint: &SdkPubkey, token_program: &SdkPubkey) -> SdkPubkey {
    SdkPubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &spl_ata_program_id(),
    )
    .0
}

fn build_create_ata_idempotent_ix(
    payer: &SdkPubkey,
    wallet: &SdkPubkey,
    mint: &SdkPubkey,
    token_program: &SdkPubkey,
) -> Instruction {
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
        data: vec![1], // Idempotent variant
    }
}

// ============================================================================
// Token-2022 mint creation
// ============================================================================

fn clone_kp(k: &Keypair) -> Keypair {
    #[allow(deprecated)]
    Keypair::from_bytes(&k.to_bytes()).unwrap()
}

fn create_mint(env: &mut Env, transfer_fee_bps: u16, decimals: u8) -> (SdkPubkey, Keypair) {
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

    let payer = clone_kp(&env.payer);
    env.send(&ixs, &[&payer, &mint]).expect("create_mint");
    (mint.pubkey(), authority)
}

fn mint_to_user(
    env: &mut Env,
    mint: &SdkPubkey,
    mint_authority: &Keypair,
    recipient: &Keypair,
    amount: u64,
) -> SdkPubkey {
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
    let payer = clone_kp(&env.payer);
    let auth = clone_kp(mint_authority);
    env.send(&[create_ata, mint_to], &[&payer, &auth])
        .expect("mint_to_user");
    ata
}

// ============================================================================
// PDA derivation + read helpers
// ============================================================================

fn derive_pool(config: &SdkPubkey, mint: &SdkPubkey) -> SdkPubkey {
    SdkPubkey::find_program_address(&[POOL_SEED, config.as_ref(), mint.as_ref()], &deep_pool::ID).0
}

fn derive_vault(pool: &SdkPubkey) -> SdkPubkey {
    SdkPubkey::find_program_address(&[VAULT_SEED, pool.as_ref()], &deep_pool::ID).0
}

fn derive_lp_mint(pool: &SdkPubkey) -> SdkPubkey {
    SdkPubkey::find_program_address(&[LP_MINT_SEED, pool.as_ref()], &deep_pool::ID).0
}

fn derive_event_authority() -> SdkPubkey {
    SdkPubkey::find_program_address(&[b"__event_authority"], &deep_pool::ID).0
}

fn token_amount(env: &Env, ata: &SdkPubkey) -> u64 {
    let acct = env.get_account(ata);
    u64::from_le_bytes(acct.data()[64..72].try_into().unwrap())
}

// ============================================================================
// Instruction wrappers
// ============================================================================

struct PoolCtx {
    mint: SdkPubkey,
    pool: SdkPubkey,
    vault: SdkPubkey,
}

fn create_pool(
    env: &mut Env,
    creator: &Keypair,
    mint: &SdkPubkey,
    token_amount: u64,
    sol_amount: u64,
) -> PoolCtx {
    let config = clone_kp(creator);
    let pool = derive_pool(&config.pubkey(), mint);
    let vault = derive_vault(&pool);
    let lp_mint = derive_lp_mint(&pool);
    let creator_token = derive_ata(&creator.pubkey(), mint, &TOKEN_2022_PROGRAM_ID);
    let creator_lp = derive_ata(&creator.pubkey(), &lp_mint, &TOKEN_2022_PROGRAM_ID);
    let pool_lp = derive_ata(&pool, &lp_mint, &TOKEN_2022_PROGRAM_ID);

    let ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::CreatePool {
            creator: creator.pubkey(),
            sol_source: creator.pubkey(),
            config: config.pubkey(),
            token_mint: *mint,
            pool,
            token_vault: vault,
            lp_mint,
            creator_token_account: creator_token,
            creator_lp_account: creator_lp,
            pool_lp_account: pool_lp,
            token_program: TOKEN_2022_PROGRAM_ID,
            associated_token_program: spl_ata_program_id(),
            system_program: system_program::ID,
            event_authority: derive_event_authority(),
            program: deep_pool::ID,
        }
        .to_account_metas(None),
        data: deep_pool::instruction::CreatePool {
            args: deep_pool::instructions::create_pool::CreatePoolArgs {
                initial_token_amount: token_amount,
                initial_sol_amount: sol_amount,
            },
        }
        .data(),
    };
    env.send(&[ix], &[creator]).expect("create_pool");
    PoolCtx {
        mint: *mint,
        pool,
        vault,
    }
}

fn submit_swap(
    env: &mut Env,
    user: &Keypair,
    p: &PoolCtx,
    amount_in: u64,
    minimum_out: u64,
    buy: bool,
) {
    let user_token = derive_ata(&user.pubkey(), &p.mint, &TOKEN_2022_PROGRAM_ID);
    let create_ata = build_create_ata_idempotent_ix(
        &user.pubkey(),
        &user.pubkey(),
        &p.mint,
        &TOKEN_2022_PROGRAM_ID,
    );
    let swap_ix = Instruction {
        program_id: deep_pool::ID,
        accounts: deep_pool::accounts::Swap {
            user: user.pubkey(),
            sol_source: user.pubkey(),
            pool: p.pool,
            token_mint: p.mint,
            token_vault: p.vault,
            user_token_account: user_token,
            token_program: TOKEN_2022_PROGRAM_ID,
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
    env.send(&[create_ata, swap_ix], &[user]).expect("swap");
}

// ============================================================================
// AMM construction from on-chain state
// ============================================================================

fn build_amm_from_chain(env: &Env, pool_key: &SdkPubkey, vault: &SdkPubkey, mint: &SdkPubkey) -> DeepPoolAmm {
    let pool_acct = env.get_account(pool_key);
    let vault_acct = env.get_account(vault);
    let mint_acct = env.get_account(mint);

    let keyed = KeyedAccount {
        key: sdk_to_jup(pool_key),
        account: sdk_account_to_jup(&pool_acct),
        params: None,
    };
    let mut amm = DeepPoolAmm::from_keyed_account(&keyed, &AmmContext::default()).unwrap();

    let mut map = AccountMap::default();
    map.insert(sdk_to_jup(pool_key), sdk_account_to_jup(&pool_acct));
    map.insert(sdk_to_jup(vault), sdk_account_to_jup(&vault_acct));
    map.insert(sdk_to_jup(mint), sdk_account_to_jup(&mint_acct));
    amm.update(&map).unwrap();
    amm
}

// ============================================================================
// Fixture
// ============================================================================

struct LivePool {
    env: Env,
    pool: PoolCtx,
    mint_authority: Keypair,
}

fn seed_live_pool(transfer_fee_bps: u16) -> LivePool {
    let mut env = Env::new();
    let (mint, authority) = create_mint(&mut env, transfer_fee_bps, 6);
    let creator = env.new_funded(20 * LAMPORTS_PER_SOL);
    mint_to_user(&mut env, &mint, &authority, &creator, 10_000_000_000_000);
    let pool = create_pool(
        &mut env,
        &creator,
        &mint,
        1_000_000_000_000,
        10 * LAMPORTS_PER_SOL,
    );
    LivePool {
        env,
        pool,
        mint_authority: authority,
    }
}

// ============================================================================
// The actual roundtrip tests
// ============================================================================

#[test]
fn quote_matches_chain_buy_no_fee() {
    let mut fx = seed_live_pool(0);
    let amm = build_amm_from_chain(&fx.env, &fx.pool.pool, &fx.pool.vault, &fx.pool.mint);

    let amount_in = 1_000_000_000; // 1 SOL
    let quote = amm
        .quote(&QuoteParams {
            amount: amount_in,
            input_mint: WSOL_MINT,
            output_mint: sdk_to_jup(&fx.pool.mint),
            swap_mode: SwapMode::ExactIn,
            fee_mode: Default::default(),
        })
        .expect("quote");

    let user = fx.env.new_funded(5 * LAMPORTS_PER_SOL);
    let user_ata = derive_ata(&user.pubkey(), &fx.pool.mint, &TOKEN_2022_PROGRAM_ID);
    submit_swap(&mut fx.env, &user, &fx.pool, amount_in, 1, true);
    let received = token_amount(&fx.env, &user_ata);

    assert_eq!(
        quote.out_amount, received,
        "quote.out_amount ({}) must equal on-chain receipt ({})",
        quote.out_amount, received
    );
}

#[test]
fn quote_matches_chain_sell_no_fee() {
    let mut fx = seed_live_pool(0);
    let amm = build_amm_from_chain(&fx.env, &fx.pool.pool, &fx.pool.vault, &fx.pool.mint);

    let amount_in = 50_000_000_000;
    let quote = amm
        .quote(&QuoteParams {
            amount: amount_in,
            input_mint: sdk_to_jup(&fx.pool.mint),
            output_mint: WSOL_MINT,
            swap_mode: SwapMode::ExactIn,
            fee_mode: Default::default(),
        })
        .expect("quote");

    let user = fx.env.new_funded(2 * LAMPORTS_PER_SOL);
    let user_authority = clone_kp(&fx.mint_authority);
    mint_to_user(&mut fx.env, &fx.pool.mint, &user_authority, &user, amount_in);

    // Measure SOL delta on the pool side — exact (direct lamport credit, no
    // tx-fee interference). User-side SOL delta would need to subtract the
    // tx fee + rent for ATA creation, which is unrelated to swap output.
    let pool_sol_before = fx.env.balance(&fx.pool.pool);
    submit_swap(&mut fx.env, &user, &fx.pool, amount_in, 1, false);
    let pool_sol_after = fx.env.balance(&fx.pool.pool);
    let sol_out = pool_sol_before - pool_sol_after;

    assert_eq!(
        quote.out_amount, sol_out,
        "quote.out_amount ({}) must equal on-chain SOL out ({})",
        quote.out_amount, sol_out
    );
}

#[test]
fn quote_matches_chain_buy_with_transfer_fee() {
    // 1% Token-2022 transfer fee on the mint. Buy = SOL in, tokens out;
    // the user-side fee skim must be reflected in quote.out_amount.
    let mut fx = seed_live_pool(100);
    let amm = build_amm_from_chain(&fx.env, &fx.pool.pool, &fx.pool.vault, &fx.pool.mint);

    let amount_in = 1_000_000_000;
    let quote = amm
        .quote(&QuoteParams {
            amount: amount_in,
            input_mint: WSOL_MINT,
            output_mint: sdk_to_jup(&fx.pool.mint),
            swap_mode: SwapMode::ExactIn,
            fee_mode: Default::default(),
        })
        .expect("quote");

    let user = fx.env.new_funded(5 * LAMPORTS_PER_SOL);
    let user_ata = derive_ata(&user.pubkey(), &fx.pool.mint, &TOKEN_2022_PROGRAM_ID);
    submit_swap(&mut fx.env, &user, &fx.pool, amount_in, 1, true);
    let received = token_amount(&fx.env, &user_ata);

    assert_eq!(
        quote.out_amount, received,
        "fee-aware buy quote ({}) must equal on-chain receipt ({})",
        quote.out_amount, received
    );
}

#[test]
fn quote_matches_chain_sell_with_transfer_fee() {
    // 1% Token-2022 transfer fee. Sell = tokens in (skimmed on the way to
    // the vault), SOL out (no transfer fee). Quote must apply the inbound
    // skim before the pool fee.
    let mut fx = seed_live_pool(100);
    let amm = build_amm_from_chain(&fx.env, &fx.pool.pool, &fx.pool.vault, &fx.pool.mint);

    let amount_in = 50_000_000_000;
    let quote = amm
        .quote(&QuoteParams {
            amount: amount_in,
            input_mint: sdk_to_jup(&fx.pool.mint),
            output_mint: WSOL_MINT,
            swap_mode: SwapMode::ExactIn,
            fee_mode: Default::default(),
        })
        .expect("quote");

    let user = fx.env.new_funded(2 * LAMPORTS_PER_SOL);
    let user_authority = clone_kp(&fx.mint_authority);
    mint_to_user(&mut fx.env, &fx.pool.mint, &user_authority, &user, amount_in);

    let pool_sol_before = fx.env.balance(&fx.pool.pool);
    submit_swap(&mut fx.env, &user, &fx.pool, amount_in, 1, false);
    let pool_sol_after = fx.env.balance(&fx.pool.pool);
    let sol_out = pool_sol_before - pool_sol_after;

    assert_eq!(
        quote.out_amount, sol_out,
        "fee-aware sell quote ({}) must equal on-chain SOL out ({})",
        quote.out_amount, sol_out
    );
}
