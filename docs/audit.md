# DeepPool Security Audit

**Date:** June 1, 2026 (v7.0.0 fresh-eyes full re-audit + hardening · v6.0.0 oracle pass) · May 25, 2026 (v5.0.0)
**Auditor:** Claude Opus 4.8 (Anthropic), fresh-eyes adversarial re-audit of the whole AMM (swap/custody, liquidity/LP, TWAP/constraints) + the v7 hardening · prior passes Opus 4.7 + Qwen3.6-35B (local)
**Version:** 7.0.0
**Framework:** Anchor 0.32.1 / Solana 3.0
**Program ID:** `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`
**Deployment:** Devnet + Mainnet

---

## Scope

| Component | Files | Description |
|-----------|-------|-------------|
| Program | 8 source files | Constant-product AMM with signer-verified namespaces, fee compounding, and LP locks |
| Kani proofs | 25 harnesses | Swap math, LP math, fee conservation, K invariant, LP lock rates, `calc_proportional` (+ **[v7]** `calc_proportional_ceil` pool-favoring rounding), overflow-returns-`None`, TWAP `price_q64` / `accumulate_price` (incl. past-2^128 wrap exactness) |
| Proptests | 31 properties | Fuzz-verified math properties across 10,000 random cases each, + 4 TWAP, **+ [v7] 3** (swap-fee min-1 dust window, `calc_proportional_ceil` remainder rounding ×2) (see [properties.md](./properties.md)) |
| Litesvm integration | 22 tests | End-to-end exercises of all 4 instructions, Token-2022 fee handling, extension blocklist rejection, H1/M1 regression guards, a CU bench, **+ 2 TWAP** (warmup fail-closed, keeperless price tracking) |
| SDK | 2 source files + tests | Transaction builders, quote engine, PDA derivation, **+ `twap.ts` read helper** (faithful wrapping mirror) with its own no-network unit test |

---

## Findings Summary

| Severity | Count | Details |
|----------|-------|---------|
| Critical | 0 | — |
| High | 0 | — |
| Medium | 0 | — |
| Low | 0 | — |
| Informational | 11 | See below (I-8–I-11 are the v6.0.0 oracle's **consumer contract**) |

**Rating: CLEAN — No vulnerabilities found.** The v6.0.0 oracle adds no critical/high/medium/low findings. Its risk surface is a *consumer contract* (I-8–I-11): the primitive is sound, but an integrator that reads the mark without its own liquidity floor (I-8) or with un-validated reserves (I-9a) can be misled. torch satisfies the contract.

---

## Changes Since v1.0.8

Four protocol-level changes shipped since the previous audit. All are strictly defensive.

### v2.0.0 — Signer-verified namespace (pool squatting fix)

`create_pool` now takes a `config: Signer`. Pool PDA derivation changed from `["deep_pool", mint]` to `["deep_pool", config, mint]`. This eliminates a class of griefing attacks documented in [pool-namespacing.md](./pool-namespacing.md) — attackers who hold bonding-curve tokens could previously pre-create the pool PDA with garbage parameters and permanently block legitimate migration. With v2.0, each caller has an isolated namespace keyed on a pubkey they must sign for, making front-running cryptographically impossible.

### v3.0.0 — Unified swap path (CPI trust model removed)

The pre-v3 `swap` buy branch split on `user.owner == system_program::ID`:
- Wallet callers → `System.transfer` from user.
- Non-wallet callers → pool's `sol_reserve` was assumed to already include `amount_in` (i.e., caller pre-deposited via direct lamport manipulation before CPIing).

The non-wallet path trusted the caller's claim of `amount_in` without verification. Any program could CPI in with its own PDA as `user`, claim a deposit that didn't happen, and receive tokens proportional to the phantom deposit. The attack was latent — only programs whitelisted by integrators could reach the path in practice — but the trust was implicit and undocumented.

v3.0 deletes the branch. All buys go through a single `System.transfer` inside the swap handler, which is self-authenticating: the system program enforces that `from` actually holds the claimed lamports. No caller can claim what they don't have.

### v3.1.0 — `sol_source` account split

To preserve CPI-caller composability (programs that hold SOL in a program-owned PDA can't `System.transfer` from it — the system program requires `from.owner == system_program`), `Swap` gained a second account:

```rust
pub user: Signer<'info>,           // token authority, ATA owner
pub sol_source: Signer<'info>,     // SOL source (buy) / SOL sink (sell)
```

Wallet callers pass `sol_source = user`. CPI callers pass a system-owned PDA they sign for via `invoke_signed`. On sell, `sol_source` receives lamports via direct credit — owner-agnostic, so program-owned accounts are valid sinks. Both accounts are `Signer`, eliminating substitution attacks.

The split cost nothing in CU (sell path identical, buy path identical modulo account layout) and enabled torch's `vault_sol` / `torch_vault` pattern without reintroducing the v3.0 trust model.

### v3.2.0 — Explicit rent-exempt floor + add_liquidity slippage

**Swap sell path — explicit rent-exempt floor check.** The sell branch uses direct lamport manipulation (pool PDA is program-owned, can't use `System.transfer`). Added an explicit `require!(pool.lamports >= rent_exempt)` check after the SOL withdrawal, making the rent-exempt preservation invariant visible at the point of lamport manipulation. The `sol_out < sol_reserve` check at the top of the handler still provides the primary guard; this is a defense-in-depth assertion.

**Add liquidity — `min_lp_out` slippage parameter.** Added `min_lp_out: u64` to `AddLiquidityArgs`. With Token-2022 transfer fees, the net tokens received by the vault can be less than the user-specified `token_amount`, yielding fewer LP tokens than expected. The `min_lp_out` check lets users reject the transaction if LP output falls below their threshold. Existing callers should pass `0` to disable.

### v4.0.0 — Event emission via `emit_cpi!`

All four instructions (`create_pool`, `add_liquidity`, `remove_liquidity`, `swap`) now emit structured events through Anchor's `emit_cpi!` macro. Events ride on a self-CPI to the program; the payload surfaces in `inner_instructions` with the layout `[8-byte EVENT_IX_TAG_LE | 8-byte event_discriminator | borsh payload]`. This is consensus-state event emission, not log-line emission — payloads are never truncated by Solana's log size limits and are always retrievable via `getTransaction`.

Four event types: `PoolCreated`, `SwapExecuted`, `LiquidityAdded`, `LiquidityRemoved`. Each carries post-state reserves, gross/net amounts on every token leg (so off-chain consumers can recover Token-2022 transfer-fee leakage), and the canonical idempotency key `(signature, inner_ix_idx)`. See [events.md](./events.md) for field-level details.

**Breaking change.** `#[event_cpi]` auto-injects `event_authority` (PDA at `[b"__event_authority"]`) and the program itself into every emitting instruction's `Accounts` struct. Every caller must regenerate from the new IDL — the in-tree SDK was updated in lockstep.

**Behavioral surface unchanged.** Events are observability, not protocol logic. The constant-product math, fee accumulation, LP locking, and rent-exempt invariants are bit-identical to v3.2.0. The 19 proptest properties and 16 Kani harnesses pass without modification.

**CU cost.** Self-CPI overhead is roughly 1k CU per event. Measured swap CU is **24k**, comfortably below Solana's 200k per-instruction budget. No realistic ix in the program is now CU-bound.

**Implementation detail — Token-2022 fee delta measurement.** Outbound token transfers (swap buy, remove_liquidity) now reload the recipient's token account after the transfer to compute `_net` amounts (= post-balance − pre-balance). This robustly captures whatever Token-2022 extension fees siphon between sender and recipient, regardless of which extension is configured. Inbound transfers already used this pattern (`vault_before` measurement); v4 brings outbound parity.

**Boxing-driven account-frame fix.** `#[event_cpi]` adds two accounts to every ix's deserialization frame. `CreatePool` and `RemoveLiquidity` exceeded the 4096-byte BPF stack with the additional accounts; both structs are now `Box<Account<...>>` / `Box<InterfaceAccount<...>>` to push the heavy fields onto the heap. `AddLiquidity` was already boxed; `Swap` stays unboxed (frame still fits). Behaviorally identical, just a memory-layout adjustment.

### v4.2.0 — Math hygiene + audit follow-ups

Six fixes across program, math, indexer, and SDK. All strictly defensive.

**Program (`add_liquidity.rs`, `remove_liquidity.rs`):**

- **`sol_required` now derives from `net_tokens`, not the gross `args.token_amount`.** Pre-v4.2, depositors over-paid SOL by exactly the Token-2022 transfer-fee fraction for fee-bearing mints (donated to existing LPs). Now: pre-validate against worst-case (assume zero fee) for the slippage guard, perform the transfer, then compute the exact `sol_required` from the post-fee net. Provider pays SOL strictly proportional to what actually landed in the vault. Validated by litesvm regression test `add_liquidity::sol_paid_matches_net_tokens_under_transfer_fee`.
- **`remove_liquidity` minimum-reserve floor tightened** from `> 0` to `>= MIN_INITIAL_SOL && >= MIN_INITIAL_TOKENS`. Locked LP (20% creator + 7.5% per add) already made a dust-floor drain impossible in practice, but the check now matches the semantic of the `MinimumLiquidityRequired` error.

**Math (`math.rs`):**

- **New `calc_proportional(input, reserve_a, reserve_b)`** — replaces the semantic misuse of `calc_lp_redeem` for proportional deposits. Same `a*b/c` shape, but the name matches the actual caller intent so a future hardening (e.g., adding `lp_amount ≤ lp_supply` validation) doesn't silently break liquidity math.
- **`u128 → u64` truncation closed across the LP-math family.** `calc_lp_mint`, `calc_lp_redeem`, and `calc_proportional` previously silently truncated when the intermediate u128 product exceeded `u64::MAX`. Now use `u64::try_from(result).ok()` — overflow returns `None`, surfaced as `MathOverflow`. Three new Kani proofs (`verify_*_overflow_returns_none`) verify this exhaustively.

**Indexer (`domain/reserves.rs`, `01-schema.sql`):**

- **"Latest reserves" tiebreak moved from `reserve_id DESC` to `signature DESC, inner_ix_idx DESC`.** The SERIAL `reserve_id` increments per insert, which means same-slot events split across two backfill pages end up with inverted reserve_ids relative to chain order — so the "latest" pick could flip during catch-up. The UNIQUE-constrained `(signature, inner_ix_idx)` tuple is deterministic from event identity and stable across re-runs. Index updated to match.

**SDK (`packages/sdk/src/getters.ts`):**

- **`getMintTransferFeeBps(connection, mint)`** — reads the active fee from a Token-2022 mint's `TransferFeeConfig` at the current epoch (returns 0 for SPL-Token or fee-free mints).
- **`getSwapQuoteForMint(connection, mint, ...)`** — one-shot read + compute. Avoids the silent-mispricing footgun of passing the default `transferFeeBps = 0` to `getSwapQuote` for fee-bearing mints. Original `getSwapQuote` keeps the pure-compute signature with a docstring guiding wallets toward the right helper.

**Test surface added.** Litesvm integration suite (`tests/litesvm/`) — 18 tests covering all 4 instructions end-to-end, including Token-2022 fee handling and the H1 + M1 regression guards. Plus 5 new Kani proofs (3 for `calc_proportional`, 2 for sibling overflow). Total: **21 Kani proofs + 24 proptests + 18 litesvm tests + E2E**.

**Net behavioral changes.** Deposits priced fairly under Token-2022 transfer fees; `remove_liquidity` floor semantic matches its error name; LP-math overflow returns `MathOverflow` instead of silently truncating; indexer "latest reserves" stable across backfill re-runs; SDK exposes a fee-aware quote helper. No new attack surface.

### v5.0.0 — Jupiter-readiness pass

Pre-submission audit pass focused on the swap path, Token-2022 surface, and program-level idiomatic cleanups. Three of the changes are minor breaking (account list + error code), one is a behavioral gate (extension blocklist), the rest are internal hardening. SDK and IDL updated in lockstep.

**Program — `swap.rs`:**

- **Dropped `init_if_needed` from `user_token_account`.** The `payer = user, authority = user` coupling was structurally broken for CPI callers whose `user` is a program-owned PDA (system_program rejects rent funding from non-system accounts), so the init branch silently failed on the cold path for torch and other CPI integrators. Industry convention (Whirlpools, Phoenix, Raydium CLMM) requires user ATAs to pre-exist. Callers prepend `createAssociatedTokenAccountIdempotent` upstream; the SPL ATA program is idempotent so this is cheap when the ATA exists. Removes the `associated_token_program` field from the `Swap` context — minor breaking change for direct integrators (IDL + SDK regenerated). Wallets and Jupiter routes are unaffected: both already create ATAs in the outer tx.
- **Explicit rent-exempt floor assertion** added before the direct `sub_lamports` in `handle_sell`. The `sol_out < sol_reserve` precondition still provides the primary mathematical guarantee; the new pre-mutation `require!(lamports - sol_out >= rent_minimum_balance(Pool::LEN))` makes the invariant visible at the mutation site and survives any future refactor that weakens the upstream check. Failure returns `MinimumLiquidityRequired`.
- **`&mut Context<Swap>` threading** in `handle_buy` / `handle_sell` replaces the `clone() + reload()` workaround. The handlers now call `ctx.accounts.{user_token_account, token_vault}.reload()` directly. No behavior change — the clones shared underlying `AccountInfo`, so the data read was identical. Read order is cleaner and the borrow shape is correct.

**Program — explicit `token_program == TOKEN_2022_PROGRAM_ID` constraint** added to all four instruction contexts (`create_pool`, `add_liquidity`, `remove_liquidity`, `swap`). Was previously enforced implicitly via mint owner — the CPI would fail at the token program if the wrong program ID were passed. The explicit constraint short-circuits at account validation with the existing `NotToken2022` error, surfacing the failure mode at the right layer.

**Program — Token-2022 mint extension blocklist on `create_pool`.** Closes audit I-5. Mints carrying any of the following extensions are rejected at pool creation with the new `UnsupportedMintExtension` error (code 6013):

| Extension | Reason |
|---|---|
| `TransferHook` | Arbitrary code on every transfer → DoS + composability risk |
| `PermanentDelegate` | Delegate authority can transfer from any account → vault drain vector |
| `InterestBearingConfig` | Stored amount drifts over time → pool math diverges from user view |
| `MintCloseAuthority` | Mint can be closed by authority → entire pool orphaned |
| `NonTransferable` | Transfers fail → no swaps possible |
| `DefaultAccountState` | New accounts (incl. vault) could be created Frozen |
| `Pausable` | Authority can halt transfers → DoS pool |

Blocklist, not allowlist — chosen specifically because DeepPool is immutable-by-design (audit I-1, upgrade authority intended for revocation). An allowlist would permanently reject every future benign extension (Metadata, Group, etc.); a blocklist gracefully accepts new metadata/display additions while rejecting the known-malicious surface. `Confidential*` family intentionally not blocked — not live on mainnet today, and reserved as the natural gate point if/when DeepPool grows a confidential-aware code path. Implementation in `create_pool::validate_mint_extensions`, with a regression test (`tests/litesvm/create_pool::rejects_blocked_extension_mint_close_authority`) confirming a `MintCloseAuthority` mint is rejected with the expected error.

**Program — `instructions.rs` re-exports.** Each instruction module's `handler` is now `pub(crate)` instead of `pub`. The `#[allow(ambiguous_glob_reexports)]` attribute and the underlying lint are gone — the collision was the four public `handler` symbols colliding through glob re-exports at crate root. Glob re-exports are still in place because Anchor's `#[program]` macro needs the generated `__client_accounts_*` modules visible from the crate root.

**Program — `Cargo.toml`:**

- Enabled `anchor-spl` features `token_2022` + `token_2022_extensions` to bring `spl_token_2022::extension::*` types into scope for the blocklist check.
- Added `doctest = false` under `[lib]`. The `#[program]` handlers + `#[derive(Accounts)]` structs can't run outside an SBPF/Anchor context, so doctest examples would either fail to compile or be misleading.

**Performance.** Hot-path swap CU after all v5.0.0 changes (measured via litesvm with `compute_units_consumed`):

| Path | CU |
|---|---|
| Hot buy (swap-only, ATA exists) | ~23-26k |
| Hot sell (swap-only, ATA exists) | ~21-24k |
| Cold (createATA-idempotent + swap, one tx) | ~28-33k |

Range reflects litesvm keypair randomization across runs. Even at the upper bound, DeepPool sits ~40% below Raydium CPMM's hot-path swap (benchmarked at 30-40k via torch integration). The new explicit `token_program` constraint costs ~100-300 CU per ix; trade-off accepted for the explicit failure mode.

**Test surface added.** One new litesvm test (`rejects_blocked_extension_mint_close_authority`) for the extension blocklist. One new diagnostic test (`report_swap_cu`) that doubles as a CU regression guard via `Env::send_with_cu`. Total: **21 Kani proofs + 24 proptests + 20 litesvm tests + E2E**.

**Net behavioral changes.** Swap ix account list shrunk by one (no more `associated_token_program`); callers must pre-create user ATA (idempotent SPL helper); explicit Token-2022 program identity check on every ix; mints with rugpull/DoS extensions rejected at pool creation; rent-exempt floor explicitly asserted at the lamport mutation site. **No new attack surface.** SDK + IDL regenerated.

### v6.0.0 — In-pool TWAP oracle

DeepPool now maintains a **keeperless** time-weighted price oracle, advanced on every swap. Rationale: in a constant-product AMM the marginal price changes *only* on a swap, so an accumulator advanced on the swap itself never misses a move — there is nothing to sample between swaps and no crank/keeper is required. Consumers (torch_market liquidations) read a manipulation-resistant mark instead of raw spot. Full design + threat model: [twap-oracle.md](./twap-oracle.md).

**State.** `Pool` gains a live cumulative head (`cum_sol_per_tok`, `cum_tok_per_sol`, `last_cum_slot`) plus a 16-slot ring of periodic snapshots (`observations`, `obs_head`). `Pool::LEN` grows **153 → 835** bytes; `Pool` is now `Box`ed in **all four** contexts (three already were; `swap` added) so the larger `try_accounts` frame stays under the 4096-byte BPF stack limit. **Layout change — existing pools are incompatible and must be recreated (pre-mainnet for this feature).**

**Accumulation.** `Σ price_q64 × Δslot` per direction, Q64.64 fixed-point, **wrapping** (mod 2^128, Uniswap-style — a consumer recovers a window with `wrapping_sub`, exact while the window's true accumulation < 2^128). Recorded at the top of `swap::handler` from the **pre-swap** reserves — the real price that held over the interval just ended. **There is no path to inject a fabricated price:** the recorded value is derived from the live vault balance and the pool's lamports, both pinned by the address-bound accounts. `add_liquidity` / `remove_liquidity` deliberately do **not** record observations — they move both reserves proportionally (price-neutral), so the read's lazy head-extension reconstructs the unchanged price correctly.

**Read.** `Pool::read_twap_sol_per_tok(sol_reserve_now, token_reserve_now, now, lookback_slots) -> Option<u128>` — a pure, read-only method (no mutation, no CPI, no authority). **The caller picks the window;** the realized window is ≥ `lookback` (≤ `lookback + spacing`); `None` if the ring lacks that much history (fail-closed warmup). Window length is *policy* and lives with the consumer; the ring/spacing are storage + max lookback only.

**Security posture.** No new instruction, no new account in any existing context, no new error code (record surfaces overflow as `MathOverflow`; read returns `Option`). The oracle is **write-only by the validated swap path, read-only by everyone else**. Manipulation resistance is the standard TWAP property — moving the mark requires holding an off-price across the window, bleeding to arbitrage every block — reinforced by a dust-pool floor (`MIN_SPOT_RESERVE` = 5 SOL) below which observations are skipped. The four new informational findings **I-8–I-11 are the oracle's consumer contract** — the conditions an integrator must respect to read the mark safely. 3 new Kani proofs + 4 proptests + 2 litesvm tests; the SDK ships a faithful TS read helper (`readTwapSolPerTok`) with the wrapping arithmetic handled correctly + a no-network unit test. **No new attack surface on the pool itself; the residual risk is integration-side and documented.**

### v7.0.0 — Fresh-eyes full re-audit + three hardening fixes

A fresh adversarial pass over the **whole** AMM (swap/custody, liquidity/LP, TWAP/constraints), motivated by torch's deeper integration. **No critical/high/medium findings.** The constant-product invariant (rounding always favours the pool), the donation-immune derived `sol_reserve` (the marginal SOL extractable from a non-refundable donation is provably < the donation), the direct-lamport sell (double rent-guarded, balanced, no `UnbalancedInstruction`), the first-deposit inflation surface (structurally closed — empty pools only seed via `create_pool`), and the TWAP's single-tx-manipulation resistance (a price-moving swap zeroes its own observation gap) all re-verified clean. Three LOW/informational hardening items were found **and fixed in v7** (a fourth — a read-side depth floor — was considered and **rejected**, see below):

1. **`create_pool` `sol_source` separation.** Added a `sol_source: Signer` distinct from `creator`, so a protocol integrator (torch migration) funds the pool's initial SOL from a program-controlled PDA — the bonded raise never transits a user wallet. Wallet callers pass `sol_source == creator`. *Closes a torch-side migration drain at the source* (see torch audit).
2. **Swap-fee min-1 floor.** `calc_swap_fee` now charges ≥ 1 unit for any nonzero swap, closing the sub-`FEE_DENOMINATOR/SWAP_FEE_BPS` (< 400-unit) "free swap" dust window. `k` was already preserved at fee = 0, so this is dust-hardening, not an invariant fix.
3. **`add_liquidity` SOL charge rounds UP.** The deposit's SOL side was floored (rounding toward the provider by < 1 lamport/add); now ceiled (`calc_proportional_ceil`) at both the slippage bound and the charge, so rounding always favours the pool. `max_sol_amount` bounds it for the provider.
**Considered and rejected — a read-side `MIN_SPOT_RESERVE` floor on `read_twap_sol_per_tok`.** The re-audit flagged an asymmetry: the *write* path skips accumulation below the floor, but the *read* extrapolates the current price over the gap even on a sub-floor pool. A natural-looking fix is to gate the read too (return `None` when sub-floor / when the mark divides to `0`). **This was rejected** — it conflicts with a deliberate, load-bearing design property: a position underwater at a *held crashed* mark must stay liquidatable, even after pool depth collapses, or bad debt sits stranded behind a depth gate (locked by torch's `liquidation_proceeds_when_pool_thin`). The read's lazy head-extension *is* the mechanism that tracks the held crashed price; gating it (or freezing the head) would surface the stale pre-crash price and block the liquidation. The residual manipulation surface is the standard TWAP cost (hold the off-price across the lookback, bleeding to arbitrage) bounded by the depth-capped borrow size; the consumer owns its own liquidity-floor policy (**I-8**). The write-side gate already keeps the ring *history* clean.

**Security posture.** No new instruction. `create_pool` gains one account (`sol_source`); all other contexts unchanged. No new error code. 1 new Kani proof (`verify_proportional_ceil_rounds_up`) + `verify_swap_fee_threshold` updated to min-1; 3 new proptests; SDK + IDL regenerated. Still **CLEAN** — every v7 change is defensive, and the rejected read-floor is documented so it isn't re-proposed.

---

## Architecture

Four instructions, four account types, two invariants:

| Instruction | Description |
|-------------|-------------|
| `create_pool` | Init pool + vault + LP mint (no freeze authority), deposit initial liquidity, mint LP (80% to creator, 20% to pool PDA). Requires `config` signer. |
| `add_liquidity` | Proportional deposit, mint LP (92.5% to provider, 7.5% to pool PDA) |
| `remove_liquidity` | Burn LP, proportional withdrawal, minimum reserves enforced via LP lock math |
| `swap` | Buy (SOL→Token) or sell (Token→SOL) with 0.25% compounding fee |

### Namespace Model

Every pool lives in a namespace keyed on the `config` pubkey used at creation:

```
pool_address = PDA(["deep_pool", config, token_mint], program_id)
```

- **Protocol namespace** (e.g., torch): `config` is a program-derived PDA (`PDA(["torch_config"], torch_program_id)`). Only the owning program can sign for it via `invoke_signed`. No third party can create a pool in that namespace.
- **Wallet namespace**: `config` is the creator's wallet. They sign the transaction directly.
- **Cross-namespace isolation**: Pools in different namespaces are at different addresses. No interference.

### LP Lock Mechanism

On every `create_pool` and `add_liquidity`, a portion of minted LP goes to `pool_lp_account` — an ATA of the LP mint owned by the pool PDA.

| Operation | User receives | Pool PDA receives (locked) |
|-----------|--------------|---------------------------|
| `create_pool` | 80% | 20% |
| `add_liquidity` | 92.5% | 7.5% |

The pool PDA is **not a signer in any instruction's LP path**. `remove_liquidity` requires the LP burner to sign; the pool can't. The locked LP is unredeemable forever.

**Reserve floor = LP lock ratio × current reserve.** If `locked_LP / total_supply = x`, then after any sequence of `remove_liquidity` calls, the pool retains at least `x * reserve` in both SOL and tokens. The floor ratchets up with every `add_liquidity` (7.5% of the new deposit joins the locked LP). It cannot ratchet down.

For the minimum-size pool (0.1 SOL initial), the permanent floor is 0.02 SOL / 20% of initial tokens. Larger pools have proportionally larger floors.

### Account Constraints

Every account is either PDA-derived, ATA-derived, or signer-verified:

- Pool: `seeds = ["deep_pool", config, token_mint]` — one per namespace per mint
- Token vault: `seeds = ["pool_vault", pool]` — owned by pool PDA
- LP mint: `seeds = ["pool_lp_mint", pool]` — mint authority = pool PDA, **no freeze authority**
- Pool LP account: ATA of LP mint owned by pool PDA — permanently unredeemable
- User accounts: ATA enforced via `associated_token::mint` + `associated_token::authority`
- Token mint: validated as Token-2022 via owner check
- `config` (create only): must sign
- `user`, `sol_source` (swap): both must sign; may be the same account or different accounts

**No account in any context can be substituted, spoofed, or forged.**

### Security Properties

1. **Signer-verified namespaces** — pool squatting is cryptographically impossible
2. **Self-authenticating SOL transfers** — `System.transfer` enforces actual lamport movement; no CPI trust model
3. **No pool admin** — no modification, pause, or close instruction exists
4. **No extraction** — 0.25% fee compounds into pool; protocol takes 0%
5. **LP reserve floor** — 20% (creator) / 7.5% (provider) locked per deposit, compounding upward
6. **No freeze authority** — LP tokens can never be frozen
7. **K monotonic** — formally verified (Kani): K only increases
8. **LP redemption bounded** — formally verified: cannot exceed proportional share
9. **First-depositor attack mitigated** — `MIN_INITIAL_SOL` + `MIN_LIQUIDITY` floor
10. **Token-2022 fee handling** — net vault balance measured post-transfer, not input amount
11. **Token-2022 extension blocklist** — `TransferHook`, `PermanentDelegate`, `InterestBearingConfig`, `MintCloseAuthority`, `NonTransferable`, `DefaultAccountState`, `Pausable` all rejected at pool creation (v5.0.0)
12. **Explicit Token-2022 program identity** — `token_program == TOKEN_2022_PROGRAM_ID` enforced at account validation on every ix (v5.0.0)
13. **Explicit rent-exempt assertion at mutation site** — swap sell path asserts `lamports - sol_out >= rent_minimum_balance(Pool::LEN)` before `sub_lamports` (v5.0.0)
14. **Native SOL** — no WSOL wrapping/unwrapping complexity
15. **Checked arithmetic** — all math uses `checked_mul` / `checked_div` with u128 intermediaries
16. **Keeperless TWAP oracle** — advanced on every swap; in a CPMM price moves only on swaps, so the accumulator never misses a move and needs no crank (v6.0.0)
17. **Oracle prices are authentic** — observations integrate the **pre-swap** reserves (live vault balance + pool lamports, both address-bound); no caller can inject a fabricated price (v6.0.0)
18. **Oracle is read-only off the swap path** — `read_twap_sol_per_tok` is a pure method (no mutation/CPI/authority); the only writer is the validated swap handler (v6.0.0)
19. **Manipulation resistance = window length** — moving the mark requires holding an off-price across the consumer's `lookback`, bleeding to arbitrage; reinforced by the `MIN_SPOT_RESERVE` dust gate and fail-closed warmup (v6.0.0)

---

## Informational Findings

### I-1: Program is upgradeable

**Description:** The program deploys with an upgrade authority. Pool state is immutable (no modification instructions exist), but a program upgrade could theoretically change swap or redemption logic for *future* calls.

**Impact:** Low — existing pool assets are safe regardless. An upgrade could only affect future behavior. The LP lock mechanism would still bind any locked LP since the ATA ownership is on-chain state, not program code.

**Recommendation:** Consider revoking upgrade authority after stabilization.

### I-2: No event emission from deep_pool handlers

**Description:** Handlers do not emit Anchor events. Off-chain indexers must rely on transaction logs or account-state diffs.

**Impact:** Informational — no security impact. Integrators (torch) emit their own events at the layer above.

### I-3: LP lock is compounding by design

**Description:** The 7.5% lock applies on every `add_liquidity`. Repeated add/remove cycles compound: after n cycles, `0.925^n` of original deposited value retained by the LP. This is the designed incentive — "don't promote via add/remove, promote via swap volume".

**Impact:** Informational. Frontend should display a warning.

### I-4: Direct lamport donations are captured by LPs, not exploitable

**Description:** Anyone can `System.transfer` lamports directly to the pool PDA, bypassing `add_liquidity`. Because `sol_reserve = pool.lamports() - rent_exempt`, donations are immediately reflected in the next swap's pricing and in LP-redeemable value.

**Analysis:** The pattern "reserve read from live lamport balance" is sometimes flagged as an attack surface. In DeepPool's case:
- On swap: donations make quoted output larger (pool looks deeper). Attacker pays more for same tokens on buy; receives more SOL on sell — but they had to donate that SOL in the first place. Net: zero profit, donation captured by LPs via K growth.
- On `remove_liquidity`: donated SOL is distributed pro-rata to LP holders.
- K invariant: still holds, since K = reserve × reserve and both sides update.

**Impact:** Not exploitable. The pattern is intentional — it means LPs get any stray SOL donations for free. Worth documenting because a naive reviewer may mis-identify this as an oracle manipulation surface.

### I-5: Token-2022 extension compatibility — RESOLVED in v5.0.0

**Description (original):** DeepPool supported Token-2022 with transfer-fee and metadata extensions (tested end-to-end). Untested configurations included interest-bearing mints, confidential transfer extensions, transfer hooks, and permanent delegate.

**Resolution:** v5.0.0 added an explicit extension blocklist enforced at `create_pool`. Mints carrying any of `TransferHook`, `PermanentDelegate`, `InterestBearingConfig`, `MintCloseAuthority`, `NonTransferable`, `DefaultAccountState`, or `Pausable` are rejected at pool creation with `UnsupportedMintExtension`. Implementation: `create_pool::validate_mint_extensions`. Regression test: `tests/litesvm/create_pool::rejects_blocked_extension_mint_close_authority`.

**Design choice — blocklist over allowlist.** Chosen specifically because DeepPool is immutable-by-design (see I-1). An allowlist locks the protocol into permanently rejecting every future benign extension; a blocklist gracefully accepts new metadata/display additions (e.g. forthcoming spl-token-2022 releases) while still rejecting the known-malicious surface. The `Confidential*` family is intentionally not blocked — not live on mainnet today, and the natural gate point if/when DeepPool grows a confidential-aware code path.

**Status:** Closed. Documented supported / blocked extensions are now enforced at the program level, not just by integrator convention.

### I-6: Sub-400-lamport swap fee rounds to zero

**Description:** `fee = amount * 25 / 10000` (integer division). For `amount < 400`, `fee == 0`. A theoretically "free" swap at microscopic sizes.

**Analysis:** Not economically exploitable. A transaction fee is ~5,000 lamports; splitting a 1 SOL trade into 2.5 million sub-400-lamport chunks costs ~12.5 billion lamports in tx fees to save 0.0025 SOL in swap fees. Proptest confirms fee bounds and monotonicity across all u64 inputs.

**Impact:** Informational.

### I-7: `sol_source` / `user` decoupling is intentional

**Description:** The `Swap` context declares two signers. Wallet callers pass the same account twice (`sol_source = user`). CPI callers pass distinct PDAs — one as token authority, one as SOL flow target.

**Analysis:** Both fields are `Signer`, so neither can be substituted by an attacker without the corresponding signature. The decoupling is required for CPI callers that hold SOL separately from state (e.g., torch's vault architecture — token ATA authority on `torch_vault`, SOL on `vault_sol`). No security impact; it just reflects the protocol's composability model.

**Impact:** Informational. SDK defaults `sol_source = user` for wallet paths — no UX change from v1.x.

---

## Oracle Consumer Contract (v6.0.0)

> The in-pool TWAP oracle is a sound primitive, but it is a *primitive* — it ships
> with a use contract. I-8–I-11 below are the conditions an integrator must respect.
> Violating them is where money risk lives; the pool itself is not at risk. torch
> satisfies all four.

### I-8: Oracle freshness is not guaranteed below `MIN_SPOT_RESERVE` — consumer must floor liquidity

**Description:** `record_observation` skips accumulation when `sol_reserve < MIN_SPOT_RESERVE` (5 SOL) — it advances `last_cum_slot` but does not integrate the price, deliberately keeping manipulable thin-pool prices out of the cumulative. Consequence: a pool that has spent time below the floor has a *stale* head, and a read during that period lazily extends the head with the current (thin) spot over the gap.

**Analysis:** This fails *closed* in the dangerous direction for a liquidation oracle. To push a pool's SOL below 5, an attacker must drain it via sells (quadratic slippage) — and once below the floor the crashed price is not recorded, so the mark does not move down → no spurious liquidation manufactured. The reverse (a thin pool's stale-high mark) is bounded by the window and the lazy-extension caveat (I-9).

**Consumer requirement (load-bearing):** integrators MUST enforce their own minimum-liquidity gate before trusting the mark — the oracle does not promise freshness on a pool that has been below `MIN_SPOT_RESERVE`. torch enforces `MIN_POOL_SOL_LENDING` / `PoolTooThin` for exactly this.

**Recommendation:** the floor gates `sol_reserve` only, not `token_reserve`. A pool with healthy SOL but a token side drained near-empty produces an extreme, easily-skewed price that *is* recorded. Draining tokens is quadratically expensive (not a cheap manipulation), but gating on *both* reserves would close it cleanly and also bound I-10. Low-priority defense-in-depth. **Impact:** Informational.

### I-9: Lazy head-extension trusts the caller's reserves and the most-recent interval

**(a) Caller-supplied reserves.** `read_twap_sol_per_tok` extends the head to `now` using the caller-supplied `sol_reserve_now` / `token_reserve_now`. It is a pure function; it trusts what it's handed. A consumer MUST read those from the real, address-validated pool + vault accounts — a consumer that passes attacker-influenced reserves only fools itself, but it *would* mis-price. torch derives them from the `deep_pool` + `deep_pool_token_vault` accounts pinned by `address =` constraints in its liquidation contexts, so the values are authentic.

**(b) Recent-interval bias.** A reserve change that is *not* a swap — a direct SOL donation (I-4) — moves the price without recording an observation. A read shortly after extends the head with the post-donation price over the whole gap `[last_cum_slot, now]`, biasing the lazy-extension portion of the mark. The bias is bounded by `gap / window`, diluted by a longer `lookback`, and costs the attacker real, unrecoverable SOL (donations are captured by LPs). The historical portion of the window is unaffected. This is the standard "the most-recent observation is the manipulable one" property of every AMM TWAP; the window is the mitigation and the consumer's `lookback` is the dial. **Impact:** Informational.

### I-10: Windowed cumulative can overflow 2^128 only at non-reachable reserve ratios

**Description:** The mark is `wrapping_sub(cum_now, anchor.cum) / dt`, exact while the window's true accumulation stays < 2^128. `price_q64 = (sol_reserve << 64) / token_reserve`; for a pathological ratio (`sol_reserve` near `u64::MAX`, `token_reserve == 1`) the per-slot price approaches 2^128, and over a multi-slot window the cumulative wraps more than once → `wrapping_sub` returns a garbage mark.

**Analysis:** Not practically reachable. With the `MIN_SPOT_RESERVE` floor (≥ 5 SOL ≈ 2^32 lamports) and any realistic token reserve, the per-slot price is ≤ ~2^96 and the max ~8.5k-slot window cumulative ≤ ~2^110 — three orders of binary magnitude below the wrap. Reaching it needs `sol_reserve` in the 10^9-SOL range (more than exists) with `token_reserve == 1`. The single-wrap exactness *is* Kani-proven (`verify_accumulate_price_wraps_exactly`); the edge is "more than one wrap within one window." **Recommendation:** the I-8 `token_reserve` floor removes the edge entirely. **Impact:** Informational.

### I-11: `read_twap_sol_per_tok` is a public, read-only method — no privileged surface

**Description:** The read method is `pub` and callable by anyone deserializing a `Pool`. No mutation, no CPI, no authority check. Correct by construction — there is nothing to protect; a caller can only compute a price for itself and cannot affect pool state or another caller. The single trust assumption (authentic reserves) is I-9(a). **Impact:** Informational — documents the trust boundary for integrators.

---

## Attack Surface

| Vector | Defense | Status |
|--------|---------|--------|
| Pool creation squatting | Signer-verified config namespace (v2.0) | MITIGATED |
| CPI phantom-deposit attack | Unified `System.transfer` in swap (v3.0) | MITIGATED |
| `sol_source` substitution | `Signer` constraint — caller must sign | MITIGATED |
| `sol_source` = protocol PDA | Anchor rejects without valid invoke_signed | N/A |
| Price manipulation | Constant product = quadratic slippage | BY DESIGN |
| Sandwich attacks | 0.25% fee compounds into pool | MITIGATED |
| Token-2022 fee mismatch | Net vault balance measured post-transfer | MITIGATED |
| Token-2022 transfer-hook reentrancy | Hooks run post-state; v5.0.0 rejects `TransferHook` mints at create_pool | MITIGATED |
| Token-2022 permanent-delegate vault drain | v5.0.0 rejects `PermanentDelegate` mints at create_pool | MITIGATED |
| Token-2022 interest-bearing reserve drift | v5.0.0 rejects `InterestBearingConfig` mints at create_pool | MITIGATED |
| Token-2022 mint-close rugpull | v5.0.0 rejects `MintCloseAuthority` mints at create_pool | MITIGATED |
| Token-2022 pause / freeze-by-default DoS | v5.0.0 rejects `Pausable` + `DefaultAccountState` + `NonTransferable` at create_pool | MITIGATED |
| token_program substitution | v5.0.0 explicit `key() == TOKEN_2022_PROGRAM_ID` constraint on every ix | MITIGATED |
| Rounding exploits | Floor on output, u128 intermediaries | MITIGATED |
| First-depositor inflation | `MIN_LIQUIDITY` + `MIN_INITIAL_SOL` | MITIGATED |
| LP drain | LP lock floor = locked_LP / supply × reserves | MITIGATED |
| Bank run | Locked LP unredeemable, floor holds | MITIGATED |
| Pool drained past rent | `sol_reserve` subtracts rent_exempt; v5.0.0 adds explicit pre-mutation `require!` at the swap-sell lamport site | MITIGATED |
| Direct SOL donation | Captured by LPs via K growth | BY DESIGN |
| Account substitution | PDA + ATA + Signer constraints throughout | MITIGATED |
| LP token freeze | No freeze authority on LP mint | MITIGATED |
| Cross-namespace interference | Pools isolated by config | MITIGATED |
| Admin exploit | No admin exists | N/A |
| Pool PDA LP redemption | PDA can't sign as provider | MITIGATED |
| Fee evasion via trade splitting | Tx fees dominate; not economic | MITIGATED |
| TWAP fake-observation injection | Observations use **pre-swap reserves** (live vault + lamports, address-bound); no caller-supplied price | N/A |
| TWAP single-swap mark manipulation | Window dilution — must hold the off-price across the lookback, bleeding to arb | BY DESIGN |
| TWAP spurious liquidation via SOL drain | Draining SOL below `MIN_SPOT_RESERVE` freezes accumulation (skip gate) → crashed price not recorded → mark stays high | MITIGATED (fail-closed) |
| TWAP staleness on thin pool | Consumer enforces own liquidity floor (I-8); torch: `PoolTooThin` | CONSUMER CONTRACT |
| TWAP recent-interval / donation bias | Window-bounded, dilutable via `lookback`, donor SOL is forfeit to LPs (I-9b) | BY DESIGN |
| TWAP cumulative 2^128 overflow | Unreachable at realistic reserves; single-wrap exactness Kani-proven (I-10) | N/A (realistic) |
| TWAP read tampering | `read_twap_sol_per_tok` is pure/read-only — caller can only fool itself (I-11) | N/A |
| Oracle keeper outage / staleness | None — advanced on every swap; AMM price only moves on swaps | N/A (keeperless) |

---

## Formal Verification & Property Testing

Two complementary layers, both passing:

**Kani (exhaustive model checking)** — 25 proof harnesses covering swap math, LP math, fee conservation, K invariant, LP lock rates, `calc_proportional`, overflow-returns-`None` across the LP-math family, and the **TWAP oracle math** (`price_q64` exactness + none-on-zero-denom; `accumulate_price` window-difference exactness, including a deliberate past-2^128 wrap). See [verification.md](./verification.md).

**Proptest (fuzz-style property testing)** — 28 properties × 10,000 cases per property (5,000 for the composite swap-roundtrip) covering the full u64 input range, **including 4 TWAP properties** (`price_q64` none-iff-zero-denom + numerator-monotonic; `accumulate_price` single-step and sequence wrapping-difference exactness). Complements Kani's concrete exactness with broad random coverage. See [properties.md](./properties.md).

Key proven / property-tested invariants:
- Fee conservation (no leakage)
- K non-decreasing (core AMM invariant)
- K strictly increases with fee (self-deepening)
- Swap output bounded by reserve (cannot drain pool)
- LP redemption bounded
- Swap fee bounded and monotonic for all u64 inputs
- LP mint / redeem roundtrip extracts nothing
- LP lock rates: exactly 20% creator / 7.5% provider, conservation holds
- Swap roundtrip (buy then sell) extracts nothing
- **TWAP `price_q64` is exact and `None` iff the denominator is zero**
- **TWAP `accumulate_price` window difference is exact mod 2^128 (the property the mark relies on), proven correct across a deliberate wrap**

---

## Conclusion

DeepPool v6.0.0 stacks six protocol-level changes on top of the v1.0.8 baseline:

1. **Pool squatting** is cryptographically blocked by signer-verified namespaces (v2.0).
2. **CPI deposit trust** is eliminated by the unified `System.transfer` path (v3.0) and preserved composability via `sol_source` (v3.1).
3. **Event observability** lands via `emit_cpi!` (v4.0) — every state-changing instruction emits a typed payload through inner instructions, with `(signature, inner_ix_idx)` as a stable idempotency key for downstream indexers.
4. **Math hygiene + Token-2022 transfer-fee parity** (v4.2) — LP-math overflow returns `MathOverflow`; deposits priced fairly under transfer-fee mints.
5. **Jupiter-readiness hardening** (v5.0) — Token-2022 extension blocklist closes I-5; explicit `token_program` constraint and rent-exempt assertion strengthen defense-in-depth; swap account list slimmed for industry-standard ATA handling; CU profile measured at ~21-26k hot path, ~40% below Raydium CPMM.
6. **In-pool keeperless TWAP oracle** (v6.0) — a manipulation-resistant, consumer-windowed price mark advanced on every swap, with no keeper. Observations integrate the real pre-swap reserves (no injectable price), resistance comes from the consumer's lookback window, and the read path is pure/read-only with fail-closed warmup. The oracle adds **no critical/high/medium/low finding**; its residual risk is an integration-side *consumer contract* (I-8–I-11) — enforce your own liquidity floor and pass authentic reserves — which torch satisfies.

The LP lock ratchet (20% creator / 7.5% provider, compounding) enforces a permanent reserve floor proportional to deposit history — the pool can never be drained past that ratio without an `add_liquidity` call that immediately widens the ratio.

Combined with 0.25% fee compounding, no freeze authority on LP tokens, no admin or close instruction, a blocklist that refuses mints carrying rugpull / DoS extensions, and a keeperless oracle whose worst case is "returns `None`," the protocol is minimal, verifiable, permanently deep, and safe to route through. No vulnerabilities found at any severity level; the v6.0.0 oracle's only sharp edges are documented as the consumer contract.
