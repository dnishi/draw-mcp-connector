//! Glyphs for writing text. When the AI writes text with draw_shapes, characters are turned into point lists of strokes.
//!
//! The glyph data (`assets/kanjivg/glyphs.bin`) is KanjiVG (https://kanjivg.tagaini.net)
//! converted with `tools/convert_kanjivg.py`. KanjiVG is Copyright (C) 2009/2010/2011 Ulrich Apel,
//! licensed under Creative Commons Attribution-Share Alike 3.0, and the converted data is distributed under the same license
//! (`assets/kanjivg/README.md`). The format is described in the conversion script.

/// Size of the glyph frame (KanjiVG's viewBox is 109×109)
pub const EM: f64 = 109.0;
/// Coordinates in the data are doubled integers
const UNITS_PER_STEP: f64 = 2.0;

static DATA: &[u8] = include_bytes!("../assets/kanjivg/glyphs.bin");

/// A point inside the glyph frame (origin at the top left of the frame, y points down)
pub type GlyphPoint = (f64, f64);

fn u16_at(data: &[u8], at: usize) -> usize {
    u16::from_le_bytes([data[at], data[at + 1]]).into()
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// Position of the i-th index entry. The index is the character count (u32) followed by (code point, data offset) pairs
fn entry_at(i: usize) -> usize {
    4 + 8 * i
}

/// Data offset. None if there is no glyph (the index is sorted by code point, so it uses binary search)
fn find(c: char) -> Option<usize> {
    let codepoint = u32::from(c);
    let (mut low, mut high) = (0, u32_at(DATA, 0) as usize);
    while low < high {
        let middle = (low + high) / 2;
        match u32_at(DATA, entry_at(middle)).cmp(&codepoint) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return Some(u32_at(DATA, entry_at(middle) + 4) as usize),
        }
    }
    None
}

/// Strokes (polylines) of a character. None if there is no glyph
pub fn glyph(c: char) -> Option<Vec<Vec<GlyphPoint>>> {
    let mut at = find(c)?;
    let stroke_count = DATA[at];
    at += 1;
    let mut strokes = Vec::with_capacity(stroke_count.into());
    for _ in 0..stroke_count {
        let point_count = u16_at(DATA, at);
        at += 2;
        let (pairs, _) = DATA[at..at + 2 * point_count].as_chunks::<2>();
        let points = pairs
            .iter()
            .map(|&[x, y]| (f64::from(x) / UNITS_PER_STEP, f64::from(y) / UNITS_PER_STEP))
            .collect();
        at += 2 * point_count;
        strokes.push(points);
    }
    Some(strokes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kanjivg_and_extra_glyphs() {
        // 日 has 4 strokes and A has 3 (KanjiVG). ( is a single stroke added by the conversion script
        assert_eq!(glyph('日').unwrap().len(), 4);
        assert_eq!(glyph('A').unwrap().len(), 3);
        assert_eq!(glyph('(').unwrap().len(), 1);
        assert_eq!(glyph('\u{1F600}'), None);
    }

    #[test]
    fn points_stay_in_the_em_box() {
        for c in ['日', 'あ', 'ア', 'g', '。'] {
            for stroke in glyph(c).unwrap() {
                assert!(stroke.len() >= 2, "{c}: 画は 2 点以上");
                assert!(
                    stroke
                        .iter()
                        .all(|&(x, y)| (0.0..=EM).contains(&x) && (0.0..=EM).contains(&y)),
                    "{c}: 枠の中"
                );
            }
        }
    }

    #[test]
    fn index_is_sorted_by_codepoint() {
        let count = u32_at(DATA, 0) as usize;
        let codepoints: Vec<u32> = (0..count).map(|i| u32_at(DATA, entry_at(i))).collect();
        assert!(codepoints.windows(2).all(|w| w[0] < w[1]));
    }
}
