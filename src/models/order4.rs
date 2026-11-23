//! Order 4: the four bytes before, and the bits of this byte known so far, choose one count.
//! Together they are 40 bits: a hash sends them to one of 2^BITS slots, and each slot keeps, beside its count,
//! CHECK_BITS more bits of the hash. A context that finds another's check in its slot says nothing (0, no
//! opinion either way) and takes the slot over when the bit is known: a collision is not taken for evidence.
//!
//! A slot is one 32-bit word: the check on top, a count of 16 bits, and a counter of the bits seen at the
//! bottom. As in order 2, a new count moves fast (its first bit takes it two thirds of the way) and an old one
//! slowly, 1 / (seen + 1.5) of the way until the counter reaches LIMIT.
//!
//! The model gives two predictions: the count's, stretched, and the same scaled by how much the count has
//! seen, so that the mixer can weigh a settled context and a new one apart.
//!
//! Beside the counts, each context of four bytes keeps the last whole byte that followed it and how many times in
//! a row it did (a second table, one 64-bit slot per context: the four bytes themselves, the byte, the run). While
//! the bits of the byte known so far agree with that byte, two more predictions say its next bit: one learnt, a
//! probability kept for each length of run and each bit's place, and one that grows with the run. Once the bits
//! disagree, or the context has no byte yet, both are 0: doubt.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 4;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 30;

/// The table holds 2^BITS slots.
const BITS: u32 = 24;

const CHECK_BITS: u32 = 8;
const SEEN_BITS: u32 = 8;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

/// A slot nobody has taken: a check of 0, a count at one half, nothing seen.
const EMPTY: u32 = 1 << (SEEN_BITS + 15);

/// The table of last bytes holds 2^LAST_BITS slots.
const LAST_BITS: u32 = 22;

/// Runs longer than this share the learnt probability of this one.
const RUN_CAP: u32 = 15;

pub struct Model {
    slots: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
    /// The check of the context being coded, already in its place at the top of a word.
    check: u32,
    /// For each context of four bytes: the four bytes on top, the run in bits 8 to 15, the last byte at the bottom.
    lasts: Vec<u64>,
    last_slot: usize,
    /// The byte the context expects, with a leading 1 above it (256 to 511), or 0 if it expects none.
    expected: u32,
    /// The run of the expected byte, capped at RUN_CAP.
    run: u32,
    /// The probability, out of 65536, that the next bit is the expected one, for each run and bit's place.
    sure: Vec<u32>,
    /// Whether the expected byte gave a prediction for the bit being coded.
    agreed: bool,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model {
            slots: vec![EMPTY; 1 << BITS],
            rates,
            slot: 0,
            check: 0,
            lasts: vec![0; 1 << LAST_BITS],
            last_slot: 0,
            expected: 0,
            run: 0,
            sure: vec![49152; ((RUN_CAP + 1) * 8) as usize],
            agreed: false,
        }
    }

    /// The context has the last four bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let h = cx.last4.wrapping_mul(0x9E37_79B1);
            self.last_slot = (h >> (32 - LAST_BITS)) as usize;
            let slot = self.lasts[self.last_slot];
            let run = ((slot >> 8) & 0xFF) as u32;
            if run != 0 && (slot >> 32) as u32 == cx.last4 {
                self.expected = 256 | (slot & 0xFF) as u32;
                self.run = run.min(RUN_CAP);
            } else {
                self.expected = 0;
            }
        }
        self.agreed = self.expected != 0 && self.expected >> (8 - cx.bit) == cx.partial;
        if self.agreed {
            let next = (self.expected >> (7 - cx.bit)) & 1;
            let sure = self.sure[(self.run * 8 + cx.bit) as usize];
            let said = cx.scale.stretch(sure >> 4);
            let grown = (self.run as i32 * 64).min(1024);
            if next == 1 {
                out[2] = said;
                out[3] = grown;
            } else {
                out[2] = -said;
                out[3] = -grown;
            }
        } else {
            out[2] = 0;
            out[3] = 0;
        }
        let mut h = cx.last4.wrapping_mul(0x9E37_79B1).wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        self.slot = (h >> (32 - BITS)) as usize;
        h ^= h >> 13;
        self.check = h << (32 - CHECK_BITS);
        let word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            out[0] = 0;
            out[1] = 0;
            return;
        }
        let seen = (word & SEEN_MASK) as i32;
        let said = cx.scale.stretch((word >> (SEEN_BITS + 4)) & 0xFFF);
        out[0] = said;
        out[1] = said * seen / LIMIT as i32;
    }

    pub fn update(&mut self, cx: &Context, bit: u32) {
        if self.agreed {
            let next = (self.expected >> (7 - cx.bit)) & 1;
            let i = (self.run * 8 + cx.bit) as usize;
            let p = self.sure[i] as i32;
            let target = if next == bit { 65535 } else { 0 };
            self.sure[i] = (p + ((target - p) >> 5)) as u32;
        }
        if cx.bit == 7 {
            let byte = ((cx.partial << 1) | bit) & 0xFF;
            let slot = self.lasts[self.last_slot];
            let run = ((slot >> 8) & 0xFF) as u32;
            let run = if run != 0 && (slot >> 32) as u32 == cx.last4 && (slot & 0xFF) as u32 == byte {
                (run + 1).min(255)
            } else {
                1
            };
            self.lasts[self.last_slot] = ((cx.last4 as u64) << 32) | ((run as u64) << 8) | byte as u64;
        }
        let mut word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            word = EMPTY;
        }
        let seen = word & SEEN_MASK;
        let p = ((word >> SEEN_BITS) & 0xFFFF) as i32;
        let target = (bit << 16) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = self.check | ((moved as u32) << SEEN_BITS) | seen;
    }
}
