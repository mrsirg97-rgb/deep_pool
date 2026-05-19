// Litesvm integration test suite for deep_pool. One test binary, submodules
// per instruction. Mirrors the pattern used in torch_market's litesvm suite.
//
// Run with `cargo test -p deep_pool --test litesvm`.
// Requires `cargo build-sbf --manifest-path programs/deep_pool/Cargo.toml`
// to have produced target/deploy/deep_pool.so.

#[path = "litesvm/harness.rs"]
mod harness;

#[path = "litesvm/sanity.rs"]
mod sanity;

#[path = "litesvm/create_pool.rs"]
mod create_pool;

#[path = "litesvm/add_liquidity.rs"]
mod add_liquidity;

#[path = "litesvm/remove_liquidity.rs"]
mod remove_liquidity;

#[path = "litesvm/swap.rs"]
mod swap;
