//! The transform: rewrites the data into a text that is easier to model, before anything is coded, and rewrites it
//! back, exactly, after everything is decoded.
//!
//! Three rewritings, each undone exactly:
//! - a capital letter is written as a marker and the small letter: CAP before a word whose first letter alone is
//!   a capital (and before each capital of a word of mixed case), UP before a word of two letters or more that is
//!   all capitals. A word is a longest run of ASCII letters;
//! - the curly quotes and the long dash, three bytes each in UTF-8, are written as one byte each;
//! - the most frequent words are written as one byte each, after their marker if they have one: the first 128 as
//!   the bytes from 128 up, the next ones as bytes below 128 that this kind of text does not use (MORE). The
//!   list is in the order of what its words save, which follows how often they come more than how long they are.
//! A byte of the data that would be taken for one of these (1 to 8, 128 and more, or one of MORE) is written
//! after ESC.

use alloc::borrow::Cow;
use alloc::vec::Vec;

const ESC: u8 = 1;
const CAP: u8 = 2;
const UP: u8 = 3;

/// The third byte of the sequences E2 80 xx that are written as the bytes 4, 5, 6, 7 and 8.
const MARKS: [u8; 5] = [0x99, 0x98, 0x9C, 0x9D, 0x94];
const FIRST_MARK: u8 = 4;

/// The words written as one byte, the first as 128: the ones that save most on this kind of text.
const FIRST_WORD: u8 = 128;
/// The bytes of the words after the first 128, one bit each, the lowest byte for the first of them: control bytes
/// and signs that prose and verse leave unused.
const MORE: u128 = bits(0, 0) | bits(11, 31) | bits(34, 37) | bits(43, 43) | bits(60, 62) | bits(64, 64)
    | bits(92, 92) | bits(94, 94) | bits(96, 96) | bits(123, 127);
const WORDS: &str = "\
    the and you that to of with his for not in was her but have my this it him your he what she is all me had \
    will as be thou are said when which from there now they so would sir shall our good thy at on more were one \
    out come then know by here upon well how them if like we no thee man their lord say than king mr who do very \
    been little should let love some did see such may time never can much enter make or must where into think \
    could yet too old before why any these first am has an take down hath made again hand lady being about dear \
    great give mrs day away look most other house tell own don scene way up two night head ll heart life miss \
    eyes myself tis mine speak ever father us might nothing after face go young another though long says richard \
    still himself done better over hear thought";

/// The bits `from` to `to`, both included.
const fn bits(from: u32, to: u32) -> u128 {
    ((1 << (to - from + 1)) - 1) << from
}

/// The place in the list of the word a byte stands for, if it stands for one.
#[inline(always)]
fn place(b: u8) -> Option<usize> {
    if b >= FIRST_WORD {
        Some((b - FIRST_WORD) as usize)
    } else if MORE >> b & 1 != 0 {
        Some(128 + (MORE & ((1 << b) - 1)).count_ones() as usize)
    } else {
        None
    }
}

fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// The text the models are given for the data.
pub fn forward(data: &[u8]) -> Cow<'_, [u8]> {
    let words: Vec<&[u8]> = WORDS.split(' ').map(|word| word.as_bytes()).collect();
    // The byte of each word of the list, in the list's order.
    let mut codes: Vec<u8> = (FIRST_WORD..=255).collect();
    codes.extend((0..FIRST_WORD).filter(|&b| MORE >> b & 1 != 0));
    let mut out = Vec::with_capacity(data.len());
    let mut small: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if is_letter(b) {
            let mut j = i;
            let mut capitals = 0;
            while j < data.len() && is_letter(data[j]) {
                if data[j].is_ascii_uppercase() {
                    capitals += 1;
                }
                j += 1;
            }
            let word = &data[i..j];
            let marker = if capitals == 0 {
                0
            } else if capitals == word.len() && capitals >= 2 {
                UP
            } else if capitals == 1 && word[0].is_ascii_uppercase() {
                CAP
            } else {
                for &c in word {
                    if c.is_ascii_uppercase() {
                        out.push(CAP);
                    }
                    out.push(c.to_ascii_lowercase());
                }
                i = j;
                continue;
            };
            if marker != 0 {
                out.push(marker);
            }
            small.clear();
            small.extend(word.iter().map(|c| c.to_ascii_lowercase()));
            match words.iter().position(|entry| *entry == &small[..]).and_then(|found| codes.get(found)) {
                Some(&code) => out.push(code),
                None => out.extend_from_slice(&small),
            }
            i = j;
        } else if b == 0xE2 && i + 2 < data.len() && data[i + 1] == 0x80 && MARKS.contains(&data[i + 2]) {
            let which = MARKS.iter().position(|&m| m == data[i + 2]).unwrap_or(0);
            out.push(FIRST_MARK + which as u8);
            i += 3;
        } else {
            if (b >= ESC && b < FIRST_MARK + MARKS.len() as u8) || place(b).is_some() {
                out.push(ESC);
            }
            out.push(b);
            i += 1;
        }
    }
    Cow::Owned(out)
}

/// The data, from the text `forward` made of it.
pub fn inverse(text: Vec<u8>) -> Vec<u8> {
    let words: Vec<&[u8]> = WORDS.split(' ').map(|word| word.as_bytes()).collect();
    let mut out = Vec::with_capacity(text.len() + text.len() / 2);
    // The marker that waits for its letters: 0, CAP or UP.
    let mut marker = 0;
    let mut i = 0;
    while i < text.len() {
        let b = text[i];
        i += 1;
        if let Some(place) = place(b) {
            let word = match words.get(place) {
                Some(word) => *word,
                None => &[],
            };
            for (at, &c) in word.iter().enumerate() {
                out.push(if marker == UP || (marker == CAP && at == 0) { c.to_ascii_uppercase() } else { c });
            }
            marker = 0;
        } else if b.is_ascii_lowercase() {
            if marker == 0 {
                out.push(b);
            } else {
                out.push(b.to_ascii_uppercase());
                if marker == CAP {
                    marker = 0;
                }
            }
        } else {
            marker = 0;
            if b == ESC {
                if i < text.len() {
                    out.push(text[i]);
                    i += 1;
                }
            } else if b == CAP || b == UP {
                marker = b;
            } else if b >= FIRST_MARK && b < FIRST_MARK + MARKS.len() as u8 {
                out.extend_from_slice(&[0xE2, 0x80, MARKS[(b - FIRST_MARK) as usize]]);
            } else {
                out.push(b);
            }
        }
    }
    out
}
