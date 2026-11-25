//! Order 3: the three bytes before, and the bits of this byte known so far, choose one slot.
//! The four values together are 32 bits, too many for a table of their own: a multiplication spreads them
//! over 2^BITS slots, and two contexts that land on the same slot share it (nothing tells them apart).
//!
//! A slot is one 32-bit word that holds two counts, which adapt at two speeds, and a counter:
//! - on top, a fast count of 11 bits, which moves a quarter of the way towards each bit it sees: it follows
//!   what the context did lately;
//! - below it, a slow count of 15 bits, which moves fast while it is new and slower as evidence grows, as
//!   order 2's does: 1 / (seen + 1.5) of the way, until the counter reaches LIMIT and the rate stays there;
//! - at the bottom, the counter of the bits seen, 6 bits.
//!
//! The model gives both counts as predictions, stretched, and the mixer weighs them.

use alloc::vec;
use alloc::vec::Vec;

use super::order4::{moved, rates};
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The fast count moves 1 / 2^FAST of the way.
const FAST: u32 = 2;

/// The counter stops here: from then on the slow count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 63;

const SEEN_BITS: u32 = 6;
const SLOW_BITS: u32 = 15;
const FAST_SHIFT: u32 = SEEN_BITS + SLOW_BITS;
const FAST_BITS: u32 = 32 - FAST_SHIFT;

/// The table holds 2^BITS slots.
const BITS: u32 = 22;

/// An odd multiplier near 2^32 divided by the golden ratio: it sends the 2^32 contexts each to its own
/// 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

/// A slot that has seen nothing: both counts at one half.
const EMPTY: u32 = (1 << 31) | (1 << (SEEN_BITS + SLOW_BITS - 1));

pub struct Model {
    slots: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        Model { slots: vec![EMPTY; 1 << BITS], rates: rates(LIMIT), slot: 0 }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last three, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = (cx.last4 << 8) | cx.partial;
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        let word = self.slots[self.slot];
        // Both counts as probabilities out of 4096.
        out[0] = cx.scale.stretch((word >> FAST_SHIFT) << (12 - FAST_BITS));
        out[1] = cx.scale.stretch((word >> (SEEN_BITS + SLOW_BITS - 12)) & 0xFFF);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.slots[self.slot];
        let fast = word >> FAST_SHIFT;
        let fast = if bit == 1 { fast + (((1 << FAST_BITS) - fast) >> FAST) } else { fast - (fast >> FAST) };
        // The slow count and the counter move as every model's do.
        self.slots[self.slot] = (fast << FAST_SHIFT) | moved(word, bit, &self.rates, SLOW_BITS, SEEN_BITS, LIMIT);
    }
}
