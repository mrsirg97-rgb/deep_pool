// Decodes Anchor `emit_cpi!` events from transaction inner instructions.
//
// emit_cpi! serializes an event as a self-CPI where the inner-instruction data =
//     [8-byte event-instruction tag: EVENT_IX_TAG_LE (fixed across Anchor)]
//     ++ [8-byte event discriminator: sha256("event:<EventName>")[..8]]
//     ++ [borsh-encoded payload]
//
// deep_pool emits four events: PoolCreated, SwapExecuted, LiquidityAdded,
// LiquidityRemoved. The discriminators are computed once at startup via
// Discriminators::compute() and matched per inner instruction.
//
// No V1 fallback — the program ships with these events from the start.

use borsh::BorshDeserialize;
use sha2::{Digest, Sha256};

use crate::constants::EVENT_IX_TAG_LE;
use crate::contracts::{
    DeepPoolEvent, LiquidityAdded, LiquidityRemoved, PoolCreated, SwapExecuted,
};
use crate::error::DecodeError;

pub type Discriminator = [u8; 8];

pub fn event_discriminator(name: &str) -> Discriminator {
    let mut hasher = Sha256::new();
    hasher.update(format!("event:{name}").as_bytes());
    let hash = hasher.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&hash[..8]);
    out
}

#[derive(Debug, Clone, Copy)]
pub struct Discriminators {
    pub pool_created: Discriminator,
    pub swap_executed: Discriminator,
    pub liquidity_added: Discriminator,
    pub liquidity_removed: Discriminator,
}

impl Discriminators {
    pub fn compute() -> Self {
        Self {
            pool_created: event_discriminator("PoolCreated"),
            swap_executed: event_discriminator("SwapExecuted"),
            liquidity_added: event_discriminator("LiquidityAdded"),
            liquidity_removed: event_discriminator("LiquidityRemoved"),
        }
    }
}

pub fn try_decode_event(
    data: &[u8],
    discs: &Discriminators,
) -> Result<DeepPoolEvent, DecodeError> {
    if data.len() < 16 {
        return Err(DecodeError::TooShort);
    }
    if data[..8] != EVENT_IX_TAG_LE {
        return Err(DecodeError::UnknownDiscriminator);
    }

    let mut event_disc = [0u8; 8];
    event_disc.copy_from_slice(&data[8..16]);
    let mut payload = &data[16..];

    let event = if event_disc == discs.pool_created {
        DeepPoolEvent::PoolCreated(PoolCreated::deserialize(&mut payload)?)
    } else if event_disc == discs.swap_executed {
        DeepPoolEvent::SwapExecuted(SwapExecuted::deserialize(&mut payload)?)
    } else if event_disc == discs.liquidity_added {
        DeepPoolEvent::LiquidityAdded(LiquidityAdded::deserialize(&mut payload)?)
    } else if event_disc == discs.liquidity_removed {
        DeepPoolEvent::LiquidityRemoved(LiquidityRemoved::deserialize(&mut payload)?)
    } else {
        return Err(DecodeError::UnknownDiscriminator);
    };

    if !payload.is_empty() {
        return Err(DecodeError::TrailingBytes);
    }

    Ok(event)
}
