//! What every part of the program is told about where the coding stands: the bytes just before, the bits of the
//! byte being coded, and the logistic scale they all share.

use alloc::vec;

use crate::logistic::Scale;
use crate::transform;

/// The state the frame keeps and shows to the models, the mixer and the refining step.
pub struct Context {
    /// The bits of the current byte known so far, after a leading 1: 1 at a byte's start, 255 at most.
    pub partial: u32,
    /// How many bits of the current byte are known: 0 to 7.
    pub bit: u32,
    /// The last four whole bytes, the most recent in the low byte.
    pub last4: u32,
    /// The last eight whole bytes, the most recent in the low byte.
    pub last8: u64,
    /// A hash of the word being written: of the letters and apostrophes since the last other byte, 0 if none.
    pub word: u32,
    /// How many whole bytes are known.
    pub position: usize,
    /// How many whole bytes are known since the last line feed: the column of the byte being coded.
    pub column: usize,
    /// Where the line being written starts in the history, and where the line before it starts: the byte at
    /// this column of the line before is at `above + column`, if that is still before `line`.
    pub line: usize,
    pub above: usize,
    /// The column as the data has it, before the transform: a word written as one byte counts for its letters,
    /// a marker for nothing. Lines are cut by that width. And the width of the whole line before.
    pub shown: u32,
    pub above_shown: u32,
    /// What each byte of the text adds to `shown`.
    widths: [u8; 256],
    /// The logistic scale: `stretch` and `squash`.
    pub scale: Scale,
}

impl Context {
    pub fn new() -> Self {
        // A byte from 128 up is a word: the transform says how long. The markers below 4 show nothing.
        let mut widths = [1u8; 256];
        for (byte, width) in widths.iter_mut().enumerate() {
            if byte < 4 {
                *width = 0;
            } else if byte >= 128 {
                *width = transform::inverse(vec![byte as u8]).len() as u8;
            }
        }
        Context {
            partial: 1,
            bit: 0,
            last4: 0,
            last8: 0,
            word: 0,
            position: 0,
            column: 0,
            line: 0,
            above: 0,
            shown: 0,
            above_shown: 0,
            widths,
            scale: Scale::new(),
        }
    }

    /// Takes the bit that was just coded.
    pub fn push(&mut self, bit: u32) {
        self.partial = (self.partial << 1) | bit;
        self.bit += 1;
        if self.bit == 8 {
            let byte = self.partial & 0xFF;
            self.last4 = (self.last4 << 8) | byte;
            self.last8 = (self.last8 << 8) | byte as u64;
            self.word = if is_word(byte) { (self.word + byte + 1).wrapping_mul(0x2F0B_4A8D) } else { 0 };
            self.position += 1;
            if byte == 10 {
                self.above = self.line;
                self.line = self.position;
                self.column = 0;
                self.above_shown = self.shown;
                self.shown = 0;
            } else {
                self.column += 1;
                self.shown += self.widths[byte as usize] as u32;
            }
            self.partial = 1;
            self.bit = 0;
        }
    }
}

/// A byte of a word: a letter, or an apostrophe (the plain one, or the curly one as the transform writes it).
fn is_word(byte: u32) -> bool {
    (byte | 0x20).wrapping_sub(0x61) < 26 || byte == 0x27 || byte == 4
}
