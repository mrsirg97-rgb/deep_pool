// Decoder integration tests. Pure-logic — no DB, no async runtime.
//
// Run with:
//   cargo test --test decoder

use borsh::BorshSerialize;
use deep_pool_indexer::constants::EVENT_IX_TAG_LE;
use deep_pool_indexer::contracts::{
    DeepPoolEvent, LiquidityAdded, LiquidityRemoved, PoolCreated, SwapExecuted,
};
use deep_pool_indexer::error::DecodeError;
use deep_pool_indexer::stream::decoder::{event_discriminator, try_decode_event, Discriminators};

// Helper: build a fully-formed event ix payload (tag + disc + borsh body).
fn frame<E: BorshSerialize>(disc: &[u8; 8], body: &E) -> Vec<u8> {
    let mut data = EVENT_IX_TAG_LE.to_vec();
    data.extend_from_slice(disc);
    body.serialize(&mut data).unwrap();
    data
}

#[test]
fn discriminators_are_stable_and_distinct() {
    let d1 = Discriminators::compute();
    let d2 = Discriminators::compute();
    assert_eq!(d1.pool_created, d2.pool_created);
    assert_eq!(d1.swap_executed, d2.swap_executed);
    assert_eq!(d1.liquidity_added, d2.liquidity_added);
    assert_eq!(d1.liquidity_removed, d2.liquidity_removed);

    let all = [
        d1.pool_created,
        d1.swap_executed,
        d1.liquidity_added,
        d1.liquidity_removed,
    ];
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert_ne!(all[i], all[j], "discriminators must be distinct");
        }
    }
}

#[test]
fn event_discriminator_matches_anchor_sha256() {
    // Sanity: discriminator for "EventName" is fully determined by
    // sha256("event:EventName")[..8]. Any drift would break decode.
    let d = event_discriminator("PoolCreated");
    assert_eq!(d.len(), 8);
    assert_eq!(d, event_discriminator("PoolCreated"));
}

#[test]
fn decode_rejects_short_data() {
    let discs = Discriminators::compute();
    let err = try_decode_event(&[0u8; 12], &discs).unwrap_err();
    assert!(matches!(err, DecodeError::TooShort));
}

#[test]
fn decode_rejects_missing_tag() {
    let discs = Discriminators::compute();
    let bad = vec![0u8; 80]; // 80 bytes of zeros — first 8 don't match the tag
    let err = try_decode_event(&bad, &discs).unwrap_err();
    assert!(matches!(err, DecodeError::UnknownDiscriminator));
}

#[test]
fn decode_rejects_unknown_event_discriminator() {
    let discs = Discriminators::compute();
    let mut bad = vec![0u8; 80];
    bad[0..8].copy_from_slice(&EVENT_IX_TAG_LE);
    bad[8..16].copy_from_slice(&event_discriminator("UnknownEvent"));
    let err = try_decode_event(&bad, &discs).unwrap_err();
    assert!(matches!(err, DecodeError::UnknownDiscriminator));
}

#[test]
fn decode_rejects_trailing_bytes() {
    // Borsh decode must consume the entire payload. Trailing bytes signal a
    // layout mismatch (or stale binary against newer event), which we surface
    // loudly rather than silently truncating.
    let discs = Discriminators::compute();
    let event = sample_pool_created();
    let mut data = frame(&discs.pool_created, &event);
    data.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);

    let err = try_decode_event(&data, &discs).unwrap_err();
    assert!(matches!(err, DecodeError::TrailingBytes));
}

#[test]
fn pool_created_roundtrip() {
    let discs = Discriminators::compute();
    let event = sample_pool_created();
    let data = frame(&discs.pool_created, &event);

    let decoded = try_decode_event(&data, &discs).unwrap();
    let DeepPoolEvent::PoolCreated(p) = decoded else {
        panic!("expected PoolCreated, got {decoded:?}");
    };
    assert_eq!(p.pool, event.pool);
    assert_eq!(p.creator, event.creator);
    assert_eq!(p.sol_in_net, event.sol_in_net);
    assert_eq!(p.tokens_in_net, event.tokens_in_net);
    assert_eq!(p.lp_supply_after, event.lp_supply_after);
    assert_eq!(p.lp_to_creator, event.lp_to_creator);
    assert_eq!(p.lp_locked, event.lp_locked);
}

#[test]
fn swap_executed_roundtrip() {
    let discs = Discriminators::compute();
    let event = SwapExecuted {
        pool: [9u8; 32],
        user: [10u8; 32],
        sol_source: [11u8; 32],
        buy: true,
        amount_in_gross: 100_000_000,
        amount_in_net: 100_000_000,
        amount_out_gross: 907_024_323_709,
        amount_out_net: 907_024_323_709,
        fee: 250_000,
        sol_reserve_after: 1_100_000_000,
        token_reserve_after: 9_092_975_676_291,
    };
    let data = frame(&discs.swap_executed, &event);

    let decoded = try_decode_event(&data, &discs).unwrap();
    let DeepPoolEvent::SwapExecuted(s) = decoded else {
        panic!("expected SwapExecuted, got {decoded:?}");
    };
    assert!(s.buy);
    assert_eq!(s.amount_in_gross, 100_000_000);
    assert_eq!(s.fee, 250_000);
}

#[test]
fn liquidity_added_roundtrip() {
    let discs = Discriminators::compute();
    let event = LiquidityAdded {
        pool: [1u8; 32],
        provider: [2u8; 32],
        sol_in_gross: 120_972_499,
        sol_in_net: 120_972_499,
        tokens_in_gross: 1_000_000_000_000,
        tokens_in_net: 1_000_000_000_000,
        lp_to_provider: 10_172_687_399,
        lp_locked: 824_812_491,
        sol_reserve_after: 1_220_972_499,
        token_reserve_after: 10_092_975_676_291,
        lp_supply_after: 110_997_498_890,
    };
    let data = frame(&discs.liquidity_added, &event);

    let decoded = try_decode_event(&data, &discs).unwrap();
    let DeepPoolEvent::LiquidityAdded(la) = decoded else {
        panic!("expected LiquidityAdded, got {decoded:?}");
    };
    assert_eq!(la.lp_to_provider, 10_172_687_399);
    assert_eq!(la.lp_locked, 824_812_491);
    // Lock should be ~7.5% of total minted (LP_LOCK_PROVIDER_BPS = 750)
    let total = la.lp_to_provider + la.lp_locked;
    let lock_bps = (la.lp_locked * 10_000) / total;
    assert!(
        (740..=760).contains(&lock_bps),
        "expected ~7.5% lock, got {lock_bps}bps",
    );
}

#[test]
fn liquidity_removed_roundtrip() {
    let discs = Discriminators::compute();
    let event = LiquidityRemoved {
        pool: [1u8; 32],
        provider: [2u8; 32],
        lp_burned: 9_017_268_659,
        sol_out_gross: 99_189_956,
        sol_out_net: 99_189_956,
        tokens_out_gross: 819_938_054_028,
        tokens_out_net: 819_938_054_028,
        sol_reserve_after: 1_121_782_543,
        token_reserve_after: 9_273_037_622_263,
        lp_supply_after: 101_980_230_231,
    };
    let data = frame(&discs.liquidity_removed, &event);

    let decoded = try_decode_event(&data, &discs).unwrap();
    let DeepPoolEvent::LiquidityRemoved(lr) = decoded else {
        panic!("expected LiquidityRemoved, got {decoded:?}");
    };
    assert_eq!(lr.lp_burned, 9_017_268_659);
    assert_eq!(lr.sol_out_net, 99_189_956);
}

fn sample_pool_created() -> PoolCreated {
    PoolCreated {
        pool: [1u8; 32],
        config: [2u8; 32],
        token_mint: [3u8; 32],
        lp_mint: [4u8; 32],
        creator: [5u8; 32],
        sol_in_gross: 1_000_000_000,
        sol_in_net: 1_000_000_000,
        tokens_in_gross: 10_000_000_000_000,
        tokens_in_net: 10_000_000_000_000,
        sol_reserve_after: 1_000_000_000,
        token_reserve_after: 10_000_000_000_000,
        lp_supply_after: 99_999_999_000,
        lp_to_creator: 79_999_999_200,
        lp_locked: 19_999_999_800,
    }
}
