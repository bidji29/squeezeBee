//! The match model: where the last bytes were last seen, and the byte that followed them there.
//!
//! An index sends a hash of the last SEED bytes to the place just after their last occurrence. While no match
//! is held, each new byte looks its place up; a place found is checked by walking back from it, which gives the
//! true length of the match (up to REACH). While a match is held, each new byte either agrees with the byte
//! the match foretold, and the match grows by one, or does not, and the match is dropped.
//!
//! During a byte, as long as the bits coded agree with the foretold byte, the model says the bit that byte
//! has next. How sure it is, is learnt: a count of how often the foretold bit came true, chosen by the length
//! of the match (in a few classes) and by how many bits of this byte have already agreed. When there is no
//! match, or the byte has already parted from the foretold one, it says nothing (0).
//!
//! It gives two predictions: the learnt one, and one from a count chosen by the foretold byte itself and the
//! byte before, so that the mixer can tell a repeat that usually holds here from one that usually breaks.
//!
//! The history is the one the frame gives to `predict` (the whole bytes coded so far): the model keeps the
//! index and no second copy of the bytes.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// How many of the last bytes are hashed to find an earlier place.
const SEED: usize = 7;

/// The index holds 2^BITS places.
const BITS: u32 = 22;

/// A match is not measured beyond this length.
const REACH: usize = 400;

/// The classes of length.
const CLASSES: usize = 32;

/// Each bit moves a count 1 / 2^RATE of the way.
const RATE: u32 = 6;

/// The second count's table: 2^SIDE_BITS counts.
const SIDE_BITS: u32 = 16;

pub struct Model {
    index: Vec<u32>,
    /// How often the foretold bit came true, out of 65536: for each class of length, for each bit of a byte.
    sure: Vec<u16>,
    /// The same, chosen by the foretold byte, the byte before and the bit.
    side: Vec<u16>,
    /// The place in the history of the byte foretold, and the length of the match (0: none).
    place: usize,
    length: usize,
    /// The byte foretold, with a leading 1 (0 when there is no match).
    foretold: u32,
    /// For the bit being coded: the bit foretold (2: none), and the two counts it was read from.
    expected: u32,
    slot: usize,
    side_slot: usize,
}

fn class(length: usize) -> usize {
    if length < 16 {
        length
    } else if length < 32 {
        16 + (length - 16) / 4
    } else if length < 96 {
        20 + (length - 32) / 16
    } else {
        (24 + (length - 96) / 40).min(CLASSES - 1)
    }
}

impl Model {
    pub fn new() -> Self {
        Model {
            index: vec![0; 1 << BITS],
            sure: vec![49152; CLASSES * 8],
            side: vec![49152; 1 << SIDE_BITS],
            place: 0,
            length: 0,
            foretold: 0,
            expected: 2,
            slot: 0,
            side_slot: 0,
        }
    }

    /// At the start of a byte: moves the match on, or looks for one, and notes the place of the last bytes.
    fn step(&mut self, cx: &Context, history: &[u8]) {
        let n = history.len();
        if self.length > 0 {
            if history[self.place] == history[n - 1] {
                self.place += 1;
                if self.length < REACH {
                    self.length += 1;
                }
            } else {
                self.length = 0;
            }
        }
        if n >= SEED {
            let seed = cx.last8 & ((1u64 << (8 * SEED)) - 1);
            let slot = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - BITS)) as usize;
            if self.length == 0 {
                let found = self.index[slot] as usize;
                if found > 0 {
                    let mut length = 0;
                    while length < REACH && length < found && history[found - 1 - length] == history[n - 1 - length] {
                        length += 1;
                    }
                    if length >= SEED {
                        self.length = length;
                        self.place = found;
                    }
                }
            }
            self.index[slot] = n as u32;
        }
        self.foretold = if self.length > 0 { history[self.place] as u32 | 256 } else { 0 };
    }

    pub fn predict(&mut self, cx: &Context, history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            self.step(cx, history);
        }
        let bit = cx.bit;
        if self.foretold >> (8 - bit) != cx.partial {
            self.expected = 2;
            out[0] = 0;
            out[1] = 0;
            return;
        }
        self.expected = (self.foretold >> (7 - bit)) & 1;
        self.slot = class(self.length) * 8 + bit as usize;
        let h = ((self.foretold & 0xFF) << 8 | (cx.last4 & 0xFF)).wrapping_mul(0x9E37_79B1) >> (32 - SIDE_BITS + 3);
        self.side_slot = ((h << 3) | bit) as usize;
        let first = cx.scale.stretch(self.sure[self.slot] as u32 >> 4);
        let second = cx.scale.stretch(self.side[self.side_slot] as u32 >> 4);
        if self.expected == 1 {
            out[0] = first;
            out[1] = second;
        } else {
            out[0] = -first;
            out[1] = -second;
        }
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        if self.expected == 2 {
            return;
        }
        let came = bit == self.expected;
        for count in [&mut self.sure[self.slot], &mut self.side[self.side_slot]] {
            let p = *count as u32;
            *count = if came { p + ((65535 - p) >> RATE) } else { p - (p >> RATE) } as u16;
        }
    }
}
