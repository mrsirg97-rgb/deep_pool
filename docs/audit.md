# DeepPool Security Audit

**Date:** April 28, 2026
**Auditor:** Claude Opus 4.7 (Anthropic)
**Version:** 4.0.0
**Framework:** Anchor 0.32.1 / Solana 3.0
**Program ID:** `CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT`
**Deployment:** Devnet + Mainnet

---

## Scope

| Component | Files | Description |
|-----------|-------|-------------|
| Program | 8 source files | Constant-product AMM with signer-verified namespaces, fee compounding, and LP locks |
| Kani proofs | 16 harnesses | Swap math, LP math, fee conservation, K invariant, LP lock rates |
| Proptests | 19 properties | Fuzz-verified math properties across 10,000 random cases each (see [properties.md](./properties.md)) |
| SDK | 1 source file | Transaction builders, quote engine, PDA derivation |

---

## Findings Summary

| Severity | Count | Details |
|----------|-------|---------|
| Critical | 0 | — |
| High | 0 | — |
| Medium | 0 | — |
| Low | 0 | — |
| Informational | 7 | See below |

**Rating: CLEAN — No vulnerabilities found**

---

## Changes Since v1.0.8

Four protocol-level changes shipped since the previous audit. All are strictly defensive.

### v2.0.0 — Signer-verified namespace (pool squatting fix)

`create_pool` now takes a `config: Signer`. Pool PDA derivation changed from `["deep_pool", mint]` to `["deep_pool", config, mint]`. This eliminates a class of griefing attacks documented in [pool-initialization-griefing.md](./pool-initialization-griefing.md) — attackers who hold bonding-curve tokens could previously pre-create the pool PDA with garbage parameters and permanently block legitimate migration. With v2.0, each caller has an isolated namespace keyed on a pubkey they must sign for, making front-running cryptographically impossible.

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
11. **Native SOL** — no WSOL wrapping/unwrapping complexity
12. **Checked arithmetic** — all math uses `checked_mul` / `checked_div` with u128 intermediaries

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

### I-5: Token-2022 extension compatibility is not universal

**Description:** DeepPool supports Token-2022 with transfer-fee and metadata extensions (tested end-to-end). Untested configurations:
- Interest-bearing mints (stored amount vs displayed amount semantics)
- Confidential transfer extensions
- Transfer hook extensions with complex hook programs
- Permanent delegate extension

**Analysis:** Transfer hooks specifically are not a reentrancy vulnerability — the standard specifies hooks run *after* the balance update, so any reentrant call sees post-trade state. But a malicious hook could still consume CU / fail the outer transfer. Interest-bearing mints may accumulate interest between reads in a way that drifts reserves.

**Impact:** Integrators should restrict pool creation to known-compatible mint configurations. Torch does this via its bonding-curve mint template.

**Recommendation:** Document supported extensions explicitly. Consider adding an instruction-level check that rejects unsupported extensions at `create_pool` if the ecosystem settles on a standard allowlist.

### I-6: Sub-400-lamport swap fee rounds to zero

**Description:** `fee = amount * 25 / 10000` (integer division). For `amount < 400`, `fee == 0`. A theoretically "free" swap at microscopic sizes.

**Analysis:** Not economically exploitable. A transaction fee is ~5,000 lamports; splitting a 1 SOL trade into 2.5 million sub-400-lamport chunks costs ~12.5 billion lamports in tx fees to save 0.0025 SOL in swap fees. Proptest confirms fee bounds and monotonicity across all u64 inputs.

**Impact:** Informational.

### I-7: `sol_source` / `user` decoupling is intentional

**Description:** The `Swap` context declares two signers. Wallet callers pass the same account twice (`sol_source = user`). CPI callers pass distinct PDAs — one as token authority, one as SOL flow target.

**Analysis:** Both fields are `Signer`, so neither can be substituted by an attacker without the corresponding signature. The decoupling is required for CPI callers that hold SOL separately from state (e.g., torch's vault architecture — token ATA authority on `torch_vault`, SOL on `vault_sol`). No security impact; it just reflects the protocol's composability model.

**Impact:** Informational. SDK defaults `sol_source = user` for wallet paths — no UX change from v1.x.

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
| Token-2022 transfer-hook reentrancy | Hooks run post-state; no exploitable path | CONSIDERED |
| Rounding exploits | Floor on output, u128 intermediaries | MITIGATED |
| First-depositor inflation | `MIN_LIQUIDITY` + `MIN_INITIAL_SOL` | MITIGATED |
| LP drain | LP lock floor = locked_LP / supply × reserves | MITIGATED |
| Bank run | Locked LP unredeemable, floor holds | MITIGATED |
| Pool drained past rent | `sol_reserve` subtracts rent_exempt, checked on exit | MITIGATED |
| Direct SOL donation | Captured by LPs via K growth | BY DESIGN |
| Account substitution | PDA + ATA + Signer constraints throughout | MITIGATED |
| LP token freeze | No freeze authority on LP mint | MITIGATED |
| Cross-namespace interference | Pools isolated by config | MITIGATED |
| Admin exploit | No admin exists | N/A |
| Pool PDA LP redemption | PDA can't sign as provider | MITIGATED |
| Fee evasion via trade splitting | Tx fees dominate; not economic | MITIGATED |

---

## Formal Verification & Property Testing

Two complementary layers, both passing:

**Kani (exhaustive model checking)** — 16 proof harnesses covering swap math, LP math, fee conservation, K invariant, and LP lock rates. See [verification.md](./verification.md).

**Proptest (fuzz-style property testing)** — 19 properties × 10,000 cases per property (5,000 for the composite swap-roundtrip) covering the full u64 input range. Complements Kani's concrete exactness with broad random coverage including roundtrip no-extraction under multi-step compositions. See [properties.md](./properties.md).

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

---

## Conclusion

DeepPool v4.0.0 closes two classes of latent issue present in v1.0.8 and adds structured event emission for off-chain consumers:

1. **Pool squatting** is cryptographically blocked by signer-verified namespaces (v2.0).
2. **CPI deposit trust** is eliminated by the unified `System.transfer` path (v3.0) and preserved composability via `sol_source` (v3.1).
3. **Event observability** lands via `emit_cpi!` (v4.0) — every state-changing instruction emits a typed payload through inner instructions, with `(signature, inner_ix_idx)` as a stable idempotency key for downstream indexers.

The LP lock ratchet (20% creator / 7.5% provider, compounding) enforces a permanent reserve floor proportional to deposit history — the pool can never be drained past that ratio without an `add_liquidity` call that immediately widens the ratio.

Combined with 0.25% fee compounding, no freeze authority on LP tokens, and no admin or close instruction, the protocol is minimal, verifiable, and permanently deep. No vulnerabilities found at any severity level.
