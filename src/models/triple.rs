//! The triple model: the two words before (their hashes, which the context keeps), the word being written so far
//! (its hash) and the bits of this byte known so far choose one count. The pair model knows one word before; after
//! "I pray you" or "said Mr. " this one has seen what follows the two words, where the pair model mixes every
//! word that ever followed the last one.
//!
//! The slots are the pair model's: a slot keeps a check of more bits of the hash beside its count, and a context
//! that finds another's check says nothing and takes the slot over. Triples of words are many more than pairs and
//! most are seen once or twice, so a count says little until it has seen a few bits: as in the pair model it gives
//! two predictions, the count's, stretched, and the same scaled by how much the count has seen. The table is the
//! pair model's size, 2^24 slots of 4 bytes.

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

    /// The context has the three hashes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let words = (cx.before2.wrapping_mul(0x7FEB_352D) ^ cx.before).rotate_left(11);
            self.base = (words.wrapping_mul(0x846C_A68B) ^ cx.word).wrapping_mul(0x9E37_79B1);
        }
        self.table.predict(self.base, cx, out);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        self.table.update(bit);
    }
}
