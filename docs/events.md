# Events

## Goal

Emit a structured event on every state-changing instruction so an off-chain indexer can reconstruct price history, swap volume, fee accrual, TVL, and LP-share-value without polling pool accounts.

## Mechanism

Anchor `emit_cpi!`. Payloads ride on a self-CPI to the program and surface in `inner_instructions`. Indexers decode `[event_ix_tag | event_discriminator | borsh payload]` from the inner ix data — no log parsing.

**Why not `emit!`:** Solana log truncation drops messages over 10kb per tx and elides under load. `emit_cpi!` data lives in inner instructions, which are part of consensus state — never truncated, always retrievable via `getTransaction`.

## Breaking change

`emit_cpi!` requires `event_authority` (PDA, seeds = `[b"__event_authority"]`) and the program itself in every emitting instruction's accounts list. All four `Accounts` structs gain two accounts. Callers (in-tree SDK, sim) regenerate from the new IDL. Acceptable: no external integrators today.

## Field conventions

- **Post-state reserves only.** `sol_reserve_after` / `token_reserve_after` / `lp_supply_after` reflect values after the instruction's effects. Pre-state is recoverable from `(post − amounts)` when needed; storing both is redundant.
- **Gross/net on every token leg.** Token-2022 transfer-fee extensions can siphon value between sender and recipient. Every token amount is reported as both `_gross` (signed/transferred amount) and `_net` (delta on the receiving account). SOL legs have gross == net but carry both fields anyway, so consumers index without conditional logic.
- **Net is measured by reload-and-delta on the recipient account.** Mirrors the existing `vault_before` pattern in `add_liquidity` and `create_pool`. Robust to any future Token-2022 extension that mutates transfer amounts.
- **Pool pubkey on every event.** Multi-pool indexers route on it without a second account read.
- **No slot, block_time, or signature.** The indexer reads them from the block header — duplicating consensus-supplied data wastes bytes.
- **Decoder contract.** Borsh field order is load-bearing. New fields go at the end; the decoder uses the metadao-challenge fallback pattern (try newest layout, fall back to prior on length mismatch).

## Events

### PoolCreated

```
PoolCreated {
    pool:                Pubkey,
    config:              Pubkey,   // namespace
    token_mint:          Pubkey,
    lp_mint:             Pubkey,
    creator:             Pubkey,
    sol_in_gross:        u64,      // = args.initial_sol_amount
    sol_in_net:          u64,      // = sol_in_gross (no SOL fee)
    tokens_in_gross:     u64,      // = args.initial_token_amount
    tokens_in_net:       u64,      // vault delta — what the pool actually got
    sol_reserve_after:   u64,
    token_reserve_after: u64,      // = tokens_in_net for a fresh pool
    lp_supply_after:     u64,      // sqrt(sol*tokens) - MIN_LIQUIDITY
    lp_to_creator:       u64,      // 80%
    lp_locked:           u64,      // 20% minted to pool PDA
}
```

Bootstraps the indexer's pool registry. One event per pool, ever.

### LiquidityAdded

```
LiquidityAdded {
    pool:                Pubkey,
    provider:            Pubkey,
    sol_in_gross:        u64,
    sol_in_net:          u64,
    tokens_in_gross:     u64,      // = args.token_amount
    tokens_in_net:       u64,      // vault delta
    lp_to_provider:      u64,      // 92.5% of mint (post 7.5% lock)
    lp_locked:           u64,      // 7.5% minted to pool PDA
    sol_reserve_after:   u64,
    token_reserve_after: u64,
    lp_supply_after:     u64,
}
```

### LiquidityRemoved

```
LiquidityRemoved {
    pool:                Pubkey,
    provider:            Pubkey,
    lp_burned:           u64,      // = args.lp_amount
    sol_out_gross:       u64,
    sol_out_net:         u64,
    tokens_out_gross:    u64,      // amount the program transferred from vault
    tokens_out_net:      u64,      // provider account delta — what they received
    sol_reserve_after:   u64,
    token_reserve_after: u64,
    lp_supply_after:     u64,
}
```

### SwapExecuted

```
SwapExecuted {
    pool:                Pubkey,
    user:                Pubkey,   // token authority
    sol_source:          Pubkey,   // = user for wallets, distinct PDA for CPI
    buy:                 bool,     // true = SOL→Token, false = Token→SOL
    amount_in_gross:     u64,      // what the user signed away (pre-fee, pre-2022)
    amount_in_net:       u64,      // pool-side delta (= AMM math input)
    amount_out_gross:    u64,      // what the program transferred (= AMM math output)
    amount_out_net:      u64,      // user-side delta (what the user received)
    fee:                 u64,      // pool fee, in input-token units (separate from Token-2022)
    sol_reserve_after:   u64,
    token_reserve_after: u64,
    total_swaps:         u64,      // pool.total_swaps after increment
}
```

`fee` is the 0.25% pool fee that compounds back into the pool. Token-2022 transfer-fee leakage is recoverable from `(gross − net)` on whichever side is the token leg.

No `lp_supply` — unchanged on swaps. Indexers carry it forward from the most recent liquidity event.

## Out of scope

- Cross-pool routing events — single-hop only today.
- Off-chain event signing or commitment — `emit_cpi!` payload is consensus-verified by virtue of being an inner instruction; no extra signing needed.
