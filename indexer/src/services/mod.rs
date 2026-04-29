pub mod context;
pub mod liquidity;
pub mod pool;
pub mod reserves;
pub mod swap;

pub use context::{Cache, RequestCtx};
pub use liquidity::LiquidityService;
pub use pool::PoolService;
pub use reserves::ReservesService;
pub use swap::SwapService;

// Lazy service accessors. Construction is free (each service is a wrapper
// holding `&mut RequestCtx`); work happens only when methods are invoked.
// Composition pattern: from inside a service, call `self.ctx.<other>()` —
// the borrow scope ends when the returned wrapper drops, freeing ctx for
// the next call.
impl RequestCtx {
    pub fn pools(&mut self) -> PoolService<'_> {
        PoolService::new(self)
    }

    pub fn reserves(&mut self) -> ReservesService<'_> {
        ReservesService::new(self)
    }

    pub fn swaps(&mut self) -> SwapService<'_> {
        SwapService::new(self)
    }

    pub fn liquidity(&mut self) -> LiquidityService<'_> {
        LiquidityService::new(self)
    }
}
