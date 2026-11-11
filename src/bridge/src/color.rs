//! Stroke colors. Stored in data as a "#rrggbb" (hex) string.

/// Color of strokes with no color specified, and of strokes from before colors existed
pub const BLACK: &str = "#000000";

/// Converts "#rrggbb" to [r, g, b]. None if the format is wrong
pub fn parse(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(parse("#ff3b30"), Some([0xff, 0x3b, 0x30]));
        assert_eq!(parse("#00FF7f"), Some([0x00, 0xff, 0x7f]));
        assert_eq!(parse(BLACK), Some([0, 0, 0]));
    }

    #[test]
    fn rejects_other_forms() {
        for bad in [
            "ff3b30", "#fff", "#ff3b3", "#ff3b300", "#gg0000", "#+f0000", "red",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
