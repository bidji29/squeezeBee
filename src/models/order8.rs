//! Order 8: the eight bytes before, and the bits of this byte known so far, choose one count.
//! Together they are 72 bits: a hash sends them to one of 2^BITS slots, and each slot keeps, beside its count,
//! CHECK_BITS more bits of the hash. A context that finds another's check in its slot says nothing (0, no
//! opinion either way) and takes the slot over when the bit is known: a collision is not taken for evidence.
//!
//! The slot is a word as in order 4: the check on top, a count of 16 bits, and a counter of the bits seen at
//! the bottom; the count moves 1 / (seen + 1.5) of the way until the counter reaches LIMIT.
//!
//! The eight bytes are hashed once a byte, when the byte starts; each bit then only mixes in the bits known so
//! far, so the work per bit is one multiplication and one look in the table.
//!
//! Two predictions, as order 4's: the count's, stretched, and the same scaled by how much the count has seen.

use super::order4::Checked;
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The table holds 2^BITS slots: many, since a context of eight bytes is rare (measured: each size from 2^16 up
/// to this one did better than the one before).
const BITS: u32 = 26;

/// The slots, their counts and their checks are order 4's (`Checked`), with LIMIT 30.
pub struct Model {
    table: Checked,
    /// What the whole bytes say of the context, worked out at a byte's start.
    base: u32,
}

impl Model {
    pub fn new() -> Self {
        Model { table: Checked::new(BITS), base: 0 }
    }

    /// The context has the last eight bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let h = cx.last8.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            self.base = ((h >> 32) ^ h) as u32;
        }
        self.table.predict(self.base, cx, out);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        self.table.update(bit);
    }
}
