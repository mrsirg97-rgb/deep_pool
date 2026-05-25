pub mod add_liquidity;
pub mod create_pool;
pub mod remove_liquidity;
pub mod swap;

// Glob-export each module so Anchor's #[program] macro can resolve the
// generated `__client_accounts_*` / `__cpi_client_accounts_*` modules from
// the crate root. The earlier `ambiguous_glob_reexports` warning was caused
// by each module exporting a public `handler` fn that collided across globs
// at crate root — those are now `pub(crate)`, so the lint is silent and we
// don't need an allow attribute.
pub use add_liquidity::*;
pub use create_pool::*;
pub use remove_liquidity::*;
pub use swap::*;
