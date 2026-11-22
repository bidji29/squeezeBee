//! The refining step: the last look at the mixed probability before the coder takes it. A step here can learn
//! that, in some context, what the mixer calls 3000 in 4096 comes true more often, or less.
//!
//! The mixed probability is stretched and cut into buckets. A table keeps, for each small context and each cut,
//! the probability that a 1 came when the mixed prediction was near that cut; what it says for a prediction is
//! the line between the two cuts around it. After each bit, the two cuts move towards the bit, each by its share
//! of the line, and fast at first, then more slowly as the cut has seen more bits.
//!
//! There are several tables, each with a context of its own (see `choose`), and one with no context at all: that
//! one says what the mixed prediction is worth everywhere, which the frame already learns for itself. So the
//! step gives the coder only what the contexts add: the average of what the tables with a context say, less what
//! the one without says.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

/// The stretched scale (4096 steps) is cut every 2^STEP steps.
const STEP: u32 = 7;
const CUTS: usize = (4096 >> STEP) + 1;

/// How many tables have a context.
const TABLES: usize = 3;

/// The bits of each table's context; the last table (index TABLES) has none.
const BITS: [u32; TABLES + 1] = [16, 20, 20, 0];

/// A cut is a probability out of 2^22 in the high bits, and in the low ten how many bits it has learnt from.
/// A cut that has seen n bits moves 1 / (n + 1.5) of the way, times its share of the line, and no more slowly
/// than 1 / LIMIT of it.
const LIMITS: [u32; TABLES + 1] = [255, 255, 255, 1023];

/// Where the counts start: a new cut is trusted as if it had seen this many bits.
const START: u32 = 2;

/// What the contexts add is given this many 64ths of its weight.
const WEIGHT: i32 = 64;

pub struct Refine {
    tables: [Vec<u32>; TABLES + 1],
    /// One over n + 1.5, out of 2^16, for each count.
    over: Vec<u32>,
    /// For the bit being coded: where each table's first cut is, and how far past it the prediction was.
    at: [usize; TABLES + 1],
    part: u32,
}

impl Refine {
    pub fn new() -> Self {
        let scale = Context::new().scale;
        let mut row = [0u32; CUTS];
        for (cut, slot) in row.iter_mut().enumerate() {
            *slot = scale.squash_fine(((cut as i32) << STEP) - 2048) << 16 | START;
        }
        let table = |bits: u32| {
            let mut t = vec![0u32; CUTS << bits];
            for chunk in t.chunks_mut(CUTS) {
                chunk.copy_from_slice(&row);
            }
            t
        };
        let mut over = vec![0u32; 1024];
        for (n, slot) in over.iter_mut().enumerate() {
            *slot = (1 << 17) / (2 * n as u32 + 3);
        }
        Refine {
            tables: [table(BITS[0]), table(BITS[1]), table(BITS[2]), table(BITS[3])],
            over,
            at: [0; TABLES + 1],
            part: 0,
        }
    }

    /// The context of each table: the bits of the current byte with the last byte, with a hash of the last two
    /// bytes, with a hash of the last three.
    fn choose(cx: &Context) -> [usize; TABLES + 1] {
        let partial = cx.partial;
        let one = (cx.last4 & 0xFF) << 8 | partial;
        let two = (cx.last4 & 0xFFFF).wrapping_mul(0x9E37_79B1).wrapping_add(partial.wrapping_mul(0xC2B2_AE35))
            >> (32 - BITS[1]);
        let three = (cx.last4 & 0xFF_FFFF).wrapping_mul(0x85EB_CA6B).wrapping_add(partial.wrapping_mul(0xC2B2_AE35))
            >> (32 - BITS[2]);
        [one as usize, two as usize, three as usize, 0]
    }

    /// The probability, out of 4096, the coder is given for the bit to come.
    pub fn refine(&mut self, cx: &Context, p: u32) -> u32 {
        let d = (cx.scale.stretch(p) + 2048) as u32;
        let cut = (d >> STEP) as usize;
        let part = d & ((1 << STEP) - 1);
        self.part = part;
        let whole = 1u32 << STEP;
        let contexts = Self::choose(cx);
        let mut said = [0i32; TABLES + 1];
        for t in 0..=TABLES {
            let at = contexts[t] * CUTS + cut;
            self.at[t] = at;
            let below = self.tables[t][at] >> 16;
            let above = self.tables[t][at + 1] >> 16;
            said[t] = ((below * (whole - part) + above * part) >> STEP) as i32;
        }
        let mut sum = 0;
        for &s in said[..TABLES].iter() {
            sum += s - said[TABLES];
        }
        // In sixteenths of a step of the probability out of 4096, which is what the frame carries over.
        let add = (sum * WEIGHT) / (TABLES as i32 * 64);
        let moved = p as i32 + (add + if add >= 0 { 8 } else { -8 }) / 16;
        moved.max(0) as u32
    }

    /// Told the bit that came, before the context moves on.
    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let whole = 1i64 << STEP;
        let part = self.part as i64;
        let target = if bit == 1 { (1i64 << 22) - 1 } else { 0 };
        for t in 0..=TABLES {
            let at = self.at[t];
            let limit = LIMITS[t];
            let table = &mut self.tables[t];
            for (slot, share) in table[at..at + 2].iter_mut().zip([whole - part, part]) {
                let count = *slot & 1023;
                let p = (*slot >> 10) as i64;
                let rate = self.over[count as usize] as i64;
                let p = p + (((target - p) * rate * share) >> (16 + STEP));
                *slot = (p as u32) << 10 | if count < limit { count + 1 } else { count };
            }
        }
    }
}
