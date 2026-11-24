//! The transform: rewrites the data into a text that is easier to model, before anything is coded, and rewrites it
//! back, exactly, after everything is decoded.
//!
//! Three rewritings, each undone exactly:
//! - a capital letter is written as a marker and the small letter: CAP before a word whose first letter alone is
//!   a capital (and before each capital of a word of mixed case), UP before a word of two letters or more that is
//!   all capitals. A word is a longest run of ASCII letters;
//! - the curly quotes and the long dash, three bytes each in UTF-8, are written as one byte each;
//! - the most frequent words are written as one byte each, from 128 up, after their marker if they have one.
//! A byte of the data that would be taken for one of these (1 to 8, or 128 and more) is written after ESC.
//!
//! Before those, the line breaks of wrapped prose are written as spaces (see `unwrap`), so that the models see
//! the words of a paragraph run on as they do inside a line.

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
const WORDS: &str = "\
    the and that with you to of have this his for not in your what was her but my which there will it \
    him would shall thou said he when she from is all little should me they had their as good more were \
    be come are then know before here upon well them never like thee enter lord than now where king so \
    think very could been sir our love nothing thy at these first on some one such another again \
    gentleman out time being richard himself about myself father great much make thought by must how \
    into if we no though man other house without say scene night heart better mr therefore who do take \
    down something hath speak made hand might micawber friend lady let";

fn is_letter(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

/// The width prose is wrapped at, in characters: a line break is put back where the next word would not fit.
const WIDTH: usize = 72;

/// Before a space that the rule would take for a line break, and before itself: the next byte is the data's own.
const KEEP: u8 = 0x1F;

/// Whether a byte starts a character (UTF-8: it is not a continuation byte).
fn starts_char(b: u8) -> bool {
    b & 0xC0 != 0x80
}

/// The rule: at a space or a line break, after a line of `line` characters, with a word of `word` characters
/// next, the line breaks there if the word does not fit.
fn breaks(line: usize, word: usize) -> bool {
    word >= 1 && line + 1 + word > WIDTH
}

/// The data with each line break that the rule gives written as a space, and each space that the rule would take
/// for a line break written after KEEP (and KEEP itself after KEEP): `wrap` puts them back exactly.
fn unwrap(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut line = 0;
    for (i, &b) in data.iter().enumerate() {
        if b == b' ' || b == b'\n' {
            let mut word = 0;
            let mut j = i + 1;
            while j < data.len() && data[j] != b' ' && data[j] != b'\n' {
                if starts_char(data[j]) {
                    word += 1;
                }
                j += 1;
            }
            let rule = breaks(line, word);
            if b == b'\n' {
                out.push(if rule { b' ' } else { b'\n' });
                line = 0;
            } else {
                if rule {
                    out.push(KEEP);
                }
                out.push(b' ');
                line += 1;
            }
        } else {
            if b == KEEP {
                out.push(KEEP);
            }
            out.push(b);
            if starts_char(b) {
                line += 1;
            }
        }
    }
    out
}

/// The data, from what `unwrap` made of it.
fn wrap(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 64);
    let mut line = 0;
    let mut i = 0;
    while i < text.len() {
        let mut b = text[i];
        i += 1;
        if b == KEEP {
            if i < text.len() {
                b = text[i];
                i += 1;
            }
        } else if b == b' ' {
            let mut word = 0;
            let mut j = i;
            while j < text.len() && text[j] != b' ' && text[j] != b'\n' {
                if text[j] == KEEP {
                    if j + 1 < text.len() && text[j + 1] == b' ' {
                        break;
                    }
                    j += 1;
                    if j < text.len() && starts_char(text[j]) {
                        word += 1;
                    }
                } else if starts_char(text[j]) {
                    word += 1;
                }
                j += 1;
            }
            if breaks(line, word) {
                b = b'\n';
            }
        }
        out.push(b);
        if b == b'\n' {
            line = 0;
        } else if starts_char(b) {
            line += 1;
        }
    }
    out
}

/// The text the models are given for the data.
pub fn forward(data: &[u8]) -> Cow<'_, [u8]> {
    let unwrapped = unwrap(data);
    let data = &unwrapped[..];
    let words: Vec<&[u8]> = WORDS.split(' ').map(|word| word.as_bytes()).collect();
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
            match words.iter().position(|entry| *entry == &small[..]) {
                Some(found) => out.push(FIRST_WORD + found as u8),
                None => out.extend_from_slice(&small),
            }
            i = j;
        } else if b == 0xE2 && i + 2 < data.len() && data[i + 1] == 0x80 && MARKS.contains(&data[i + 2]) {
            let which = MARKS.iter().position(|&m| m == data[i + 2]).unwrap_or(0);
            out.push(FIRST_MARK + which as u8);
            i += 3;
        } else {
            if (b >= ESC && b < FIRST_MARK + MARKS.len() as u8) || b >= FIRST_WORD {
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
        if b >= FIRST_WORD {
            let word = match words.get((b - FIRST_WORD) as usize) {
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
    wrap(&out)
}
