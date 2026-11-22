//! Order 3: the three bytes before, and the bits of this byte known so far, choose one count.
//! The four values together are 32 bits, too many for a table of their own: a multiplication spreads them
//! over 2^BITS counts, and two contexts that land on the same count share it (nothing tells them apart).
//! Each count is 16 bits and moves a sixteenth of the way towards each bit it sees, as order 2's do.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 1;

const RATE: u32 = 4;

/// The table holds 2^BITS counts.
const BITS: u32 = 22;

/// An odd multiplier near 2^32 divided by the golden ratio: it sends the 2^32 contexts each to its own
/// 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

pub struct Model {
    counts: Vec<u16>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        Model { counts: vec![1 << 15; 1 << BITS], slot: 0 }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last three, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = (cx.last4 << 8) | cx.partial;
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        out[0] = cx.scale.stretch((self.counts[self.slot] >> 4) as u32);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let p = self.counts[self.slot] as u32;
        let moved = if bit == 1 { p + ((65536 - p) >> RATE) } else { p - (p >> RATE) };
        self.counts[self.slot] = moved as u16;
    }
}
