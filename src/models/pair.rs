//! The pair model: the word before (its hash, which the context keeps), the word being written so far (its
//! hash) and the bits of this byte known so far choose one count. After "of " it has seen which words follow
//! "of", and after "of t" which of them start with a t; the word model knows the word so far and not the one
//! before, and the orders see the word before only when it is short enough to be in their bytes.
//!
//! The slots are order 4's kind. The context is hashed to one of 2^BITS slots, and each slot keeps, beside its
//! count, CHECK_BITS more bits of the hash: a context that finds another's check in its slot says nothing
//! (0, no opinion either way) and takes the slot over when the bit is known. Pairs of words are many and most
//! are seen a few times: a collision taken for evidence would cost more here than anywhere.
//!
//! A slot is one 32-bit word: the check on top, a count of 16 bits, and a counter of the bits seen at the
//! bottom. A new count moves fast and an old one slowly, 1 / (seen + 1.5) of the way until the counter reaches
//! LIMIT. As in order 4 the model gives two predictions: the count's, stretched, and the same scaled by how
//! much the count has seen.

use super::order4::Checked;
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The table holds 2^BITS slots.
const BITS: u32 = 24;

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

    /// The context has the two hashes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            self.base = (cx.before.rotate_left(15) ^ cx.word).wrapping_mul(0x9E37_79B1);
        }
        self.table.predict(self.base, cx, out);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        self.table.update(bit);
    }
}
