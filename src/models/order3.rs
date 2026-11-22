//! Order 3: the three bytes before, and the bits of this byte known so far, choose one slot.
//! The four values together are 32 bits, too many for a table of their own: a multiplication spreads them
//! over 2^BITS slots, and two contexts that land on the same slot share it (nothing tells them apart).
//!
//! A slot is one 64-bit word that holds two counts, which adapt at two speeds at once, and a counter of the
//! bits seen (8 bits, at the bottom). Both counts move fast while the slot is new and slower as evidence
//! grows, 1 / (seen + 1.5) of the way towards each bit: the fast count (16 bits) until the counter reaches
//! FAST_LIMIT, the slow one (22 bits) until it reaches LIMIT, and each keeps its last rate from then on. The
//! model gives both as predictions, and the mixer weighs the one that follows a change against the one that
//! settles.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The counter stops here: from then on the slow count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 120;

/// From here on the fast count moves 1 / (FAST_LIMIT + 1.5) of the way.
const FAST_LIMIT: u32 = 4;

/// The table holds 2^BITS slots.
const BITS: u32 = 22;

/// An odd multiplier near 2^32 divided by the golden ratio: it sends the 2^32 contexts each to its own
/// 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

/// Both counts at one half, nothing seen: the slow count from bit 32, the fast one from bit 8.
const EMPTY: u64 = (1 << 53) | (1 << 23);

pub struct Model {
    slots: Vec<u64>,
    /// 65536 / (seen + 1.5), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { slots: vec![EMPTY; 1 << BITS], rates, slot: 0 }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last three, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = (cx.last4 << 8) | cx.partial;
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        let word = self.slots[self.slot];
        out[0] = cx.scale.stretch((word >> 42) as u32);
        out[1] = cx.scale.stretch(((word >> 12) & 0xFFF) as u32);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.slots[self.slot];
        let seen = (word & 0xFF) as u32;
        let fast = ((word >> 8) & 0xFFFF) as i32;
        let slow = (word >> 32) as i32;
        let slow_target = (bit << 22) as i32 - bit as i32;
        let fast_target = (bit << 16) as i32 - bit as i32;
        let slow = slow + (((slow_target - slow) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let fast_rate = self.rates[seen.min(FAST_LIMIT) as usize] as i32;
        let fast = fast + (((fast_target - fast) * fast_rate) >> 16);
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = ((slow as u64) << 32) | ((fast as u64) << 8) | seen as u64;
    }
}
