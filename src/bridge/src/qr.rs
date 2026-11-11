//! QR code (Model 2) generation. It only carries the connection URL, so it is limited to:
//! 8-bit byte mode, error correction level M, versions 1 to 10 (up to 213 bytes at M).
//!
//! The steps follow the encoding procedure of JIS X 0510 (ISO/IEC 18004):
//! data encoding → Reed–Solomon code → block interleaving → placement → masking → format and version information

/// Block structure per version at error correction level M
struct BlockSpec {
    /// Error correction codewords per block
    ec_per_block: usize,
    /// Pairs of (number of blocks, data codewords per block)
    groups: &'static [(usize, usize)],
}

/// From JIS X 0510 Table 9 (error correction characteristics): level M, versions 1 to 10
const LEVEL_M_BLOCKS: [BlockSpec; 10] = [
    BlockSpec {
        ec_per_block: 10,
        groups: &[(1, 16)],
    },
    BlockSpec {
        ec_per_block: 16,
        groups: &[(1, 28)],
    },
    BlockSpec {
        ec_per_block: 26,
        groups: &[(1, 44)],
    },
    BlockSpec {
        ec_per_block: 18,
        groups: &[(2, 32)],
    },
    BlockSpec {
        ec_per_block: 24,
        groups: &[(2, 43)],
    },
    BlockSpec {
        ec_per_block: 16,
        groups: &[(4, 27)],
    },
    BlockSpec {
        ec_per_block: 18,
        groups: &[(4, 31)],
    },
    BlockSpec {
        ec_per_block: 22,
        groups: &[(2, 38), (2, 39)],
    },
    BlockSpec {
        ec_per_block: 22,
        groups: &[(3, 36), (2, 37)],
    },
    BlockSpec {
        ec_per_block: 26,
        groups: &[(4, 43), (1, 44)],
    },
];

const MAX_VERSION: usize = LEVEL_M_BLOCKS.len();

/// Code of error correction level M put in the format information
const LEVEL_M_BITS: u32 = 0b00;

/// Quiet zone needed for reading (in modules)
const QUIET_ZONE: usize = 4;

fn block_spec(version: usize) -> &'static BlockSpec {
    &LEVEL_M_BLOCKS[version - 1]
}

fn data_codeword_count(version: usize) -> usize {
    block_spec(version).groups.iter().map(|(n, k)| n * k).sum()
}

fn module_size(version: usize) -> usize {
    version * 4 + 17
}

/// Center coordinates of the alignment patterns (shared by rows and columns)
fn alignment_positions(version: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let count = version / 7 + 2;
    let step = (version * 4 + 4).div_ceil(count * 2 - 2) * 2;
    let mut positions = vec![6];
    let mut pos = module_size(version) - 7;
    while positions.len() < count {
        positions.insert(1, pos);
        pos -= step;
    }
    positions
}

// ---- GF(2^8) (primitive polynomial x^8 + x^4 + x^3 + x^2 + 1) and the Reed–Solomon code ----

struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

const GF: Gf = {
    let mut exp = [0; 512];
    let mut log = [0; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Gf { exp, log }
};

fn gf_multiply(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    GF.exp[GF.log[a as usize] as usize + GF.log[b as usize] as usize]
}

/// Coefficients of the generator polynomial (x - α^0)(x - α^1)…(x - α^(degree-1)) (highest first, excluding the leading 1)
fn generator_polynomial(degree: usize) -> Vec<u8> {
    let mut poly = vec![1u8];
    for i in 0..degree {
        let mut next = vec![0u8; poly.len() + 1];
        for (j, &coefficient) in poly.iter().enumerate() {
            next[j] ^= coefficient;
            next[j + 1] ^= gf_multiply(coefficient, GF.exp[i]);
        }
        poly = next;
    }
    poly.remove(0);
    poly
}

/// Error correction codewords for the data codewords
fn reed_solomon(data: &[u8], degree: usize) -> Vec<u8> {
    let generator = generator_polynomial(degree);
    let mut remainder = vec![0u8; degree];
    for &byte in data {
        let factor = byte ^ remainder.remove(0);
        remainder.push(0);
        for (r, &coefficient) in remainder.iter_mut().zip(&generator) {
            *r ^= gf_multiply(coefficient, factor);
        }
    }
    remainder
}

// ---- Data encoding ----

#[derive(Default)]
struct BitBuffer(Vec<bool>);

impl BitBuffer {
    fn append(&mut self, value: u32, length: usize) {
        self.0
            .extend((0..length).rev().map(|i| (value >> i) & 1 == 1));
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.0
            .chunks(8)
            .map(|bits| {
                bits.iter()
                    .fold(0, |byte, &bit| (byte << 1) | u8::from(bit))
            })
            .collect()
    }
}

/// Bit count of the character count indicator in byte mode (8 for versions 1 to 9, 16 for 10 to 26)
fn count_bits(version: usize) -> usize {
    if version <= 9 { 8 } else { 16 }
}

/// Data too long to fit
#[derive(Debug, PartialEq, Eq)]
pub struct TooLong;

fn choose_version(byte_length: usize) -> Result<usize, TooLong> {
    (1..=MAX_VERSION)
        .find(|&v| 4 + count_bits(v) + byte_length * 8 <= data_codeword_count(v) * 8)
        .ok_or(TooLong)
}

/// Data codewords made of the mode indicator, character count, data, terminator, and padding
fn encode_data(bytes: &[u8], version: usize) -> Vec<u8> {
    let capacity = data_codeword_count(version);
    let mut buffer = BitBuffer::default();
    buffer.append(0b0100, 4); // 8-bit byte mode
    buffer.append(bytes.len() as u32, count_bits(version));
    for &byte in bytes {
        buffer.append(byte.into(), 8);
    }
    let terminator = 4.min(capacity * 8 - buffer.0.len());
    buffer.append(0, terminator);
    buffer.append(0, (8 - buffer.0.len() % 8) % 8);

    let mut codewords = buffer.to_bytes();
    let padding = [0xec, 0x11].into_iter().cycle();
    codewords.extend(padding.take(capacity - codewords.len()));
    codewords
}

/// Splits into blocks, adds error correction, and interleaves data and error correction column by column
fn interleave(data: &[u8], version: usize) -> Vec<u8> {
    let spec = block_spec(version);
    let mut blocks = Vec::new();
    let mut rest = data;
    for &(count, length) in spec.groups {
        for _ in 0..count {
            let (block, tail) = rest.split_at(length);
            rest = tail;
            blocks.push((block, reed_solomon(block, spec.ec_per_block)));
        }
    }

    let longest = blocks
        .iter()
        .map(|(block, _)| block.len())
        .max()
        .unwrap_or(0);
    let mut result = Vec::new();
    for i in 0..longest {
        result.extend(blocks.iter().filter_map(|(block, _)| block.get(i)));
    }
    for i in 0..spec.ec_per_block {
        result.extend(blocks.iter().map(|(_, ec)| ec[i]));
    }
    result
}

// ---- Format and version information (BCH code) ----

fn bch_remainder(value: u32, generator: u32) -> u32 {
    let degree = 31 - generator.leading_zeros();
    let mut remainder = value << degree;
    for i in (degree..32 - remainder.leading_zeros()).rev() {
        if (remainder >> i) & 1 == 1 {
            remainder ^= generator << (i - degree);
        }
    }
    remainder
}

/// Format information for error correction level M and the mask number (15 bits, masked)
fn format_bits(mask: usize) -> u32 {
    let data = (LEVEL_M_BITS << 3) | mask as u32;
    ((data << 10) | bch_remainder(data, 0x537)) ^ 0x5412
}

/// Version information (18 bits, version 7 and up)
fn version_bits(version: usize) -> u32 {
    ((version as u32) << 12) | bch_remainder(version as u32, 0x1f25)
}

// ---- Placement ----

fn mask_applies(mask: usize, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y).is_multiple_of(2),
        1 => y.is_multiple_of(2),
        2 => x.is_multiple_of(3),
        3 => (x + y).is_multiple_of(3),
        4 => (x / 3 + y / 2).is_multiple_of(2),
        5 => (x * y) % 2 + (x * y) % 3 == 0,
        6 => ((x * y) % 2 + (x * y) % 3).is_multiple_of(2),
        _ => ((x + y) % 2 + (x * y) % 3).is_multiple_of(2),
    }
}

#[derive(Clone)]
struct Matrix {
    size: usize,
    /// true for dark modules. Index is y * size + x
    modules: Vec<bool>,
    /// Function patterns (places where no data goes)
    reserved: Vec<bool>,
}

impl Matrix {
    fn new(version: usize) -> Self {
        let size = module_size(version);
        Self {
            size,
            modules: vec![false; size * size],
            reserved: vec![false; size * size],
        }
    }

    fn set_function(&mut self, x: usize, y: usize, dark: bool) {
        self.modules[y * self.size + x] = dark;
        self.reserved[y * self.size + x] = true;
    }

    fn dark(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    fn draw_finder(&mut self, cx: usize, cy: usize) {
        // 9x9 including the separator. Only the rings at distance 2 and 4 from the center are light
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                let size = self.size as i32;
                if (0..size).contains(&x) && (0..size).contains(&y) {
                    let distance = dx.abs().max(dy.abs());
                    self.set_function(x as usize, y as usize, distance != 2 && distance != 4);
                }
            }
        }
    }

    fn draw_format(&mut self, bits: u32) {
        let size = self.size;
        let bit = |i: usize| (bits >> i) & 1 == 1;
        // Around the top-left finder pattern
        for i in 0..=5 {
            self.set_function(8, i, bit(i));
        }
        self.set_function(8, 7, bit(6));
        self.set_function(8, 8, bit(7));
        self.set_function(7, 8, bit(8));
        for i in 9..15 {
            self.set_function(14 - i, 8, bit(i));
        }
        // Top right and bottom left
        for i in 0..8 {
            self.set_function(size - 1 - i, 8, bit(i));
        }
        for i in 8..15 {
            self.set_function(8, size - 15 + i, bit(i));
        }
        self.set_function(8, size - 8, true); // Always-dark module
    }

    fn draw_version(&mut self, version: usize) {
        if version < 7 {
            return;
        }
        let bits = version_bits(version);
        for i in 0..18 {
            let dark = (bits >> i) & 1 == 1;
            let a = self.size - 11 + i % 3;
            let b = i / 3;
            self.set_function(a, b, dark);
            self.set_function(b, a, dark);
        }
    }

    fn draw_function_patterns(&mut self, version: usize) {
        let size = self.size;
        for i in 0..size {
            self.set_function(6, i, i % 2 == 0);
            self.set_function(i, 6, i % 2 == 0);
        }
        self.draw_finder(3, 3);
        self.draw_finder(size - 4, 3);
        self.draw_finder(3, size - 4);

        let positions = alignment_positions(version);
        let last = positions.len().saturating_sub(1);
        for (i, &cy) in positions.iter().enumerate() {
            for (j, &cx) in positions.iter().enumerate() {
                // Skip the three positions that overlap the finder patterns
                if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                    continue;
                }
                for dy in 0..5 {
                    for dx in 0..5 {
                        let ring = (dx as i32 - 2).abs().max((dy as i32 - 2).abs());
                        self.set_function(cx + dx - 2, cy + dy - 2, ring != 1);
                    }
                }
            }
        }

        self.draw_format(0); // Reserve the area (rewritten later with the mask number)
        self.draw_version(version);
    }

    /// Returns the coordinates of data modules in placement order.
    /// Zigzags up and down two columns at a time from the bottom right, skipping the vertical timing pattern (column 6)
    fn data_positions(&self) -> Vec<(usize, usize)> {
        let size = self.size;
        let mut positions = Vec::new();
        let mut right = size - 1;
        loop {
            if right == 6 {
                right = 5;
            }
            let upward = (right + 1) & 2 == 0;
            for vertical in 0..size {
                let y = if upward {
                    size - 1 - vertical
                } else {
                    vertical
                };
                for x in [right, right - 1] {
                    if !self.reserved[y * size + x] {
                        positions.push((x, y));
                    }
                }
            }
            if right < 2 {
                return positions;
            }
            right -= 2;
        }
    }

    fn draw_codewords(&mut self, codewords: &[u8]) {
        // Remainder bits stay light
        let bits = codewords
            .iter()
            .flat_map(|&byte| (0..8).rev().map(move |i| (byte >> i) & 1 == 1));
        for ((x, y), dark) in self.data_positions().into_iter().zip(bits) {
            self.modules[y * self.size + x] = dark;
        }
    }

    fn apply_mask(&mut self, mask: usize) {
        for y in 0..self.size {
            for x in 0..self.size {
                let index = y * self.size + x;
                if !self.reserved[index] && mask_applies(mask, x, y) {
                    self.modules[index] = !self.modules[index];
                }
            }
        }
    }

    // ---- Mask evaluation (pick the one with the lowest penalty) ----

    fn penalty(&self) -> usize {
        let size = self.size;
        let rows = (0..size).map(|y| (0..size).map(|x| self.dark(x, y)).collect::<Vec<_>>());
        let columns = (0..size).map(|x| (0..size).map(|y| self.dark(x, y)).collect::<Vec<_>>());
        let mut penalty: usize = rows.chain(columns).map(|line| line_penalty(&line)).sum();

        // N2: 2x2 blocks of the same color
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let color = self.dark(x, y);
                if color == self.dark(x + 1, y)
                    && color == self.dark(x, y + 1)
                    && color == self.dark(x + 1, y + 1)
                {
                    penalty += 3;
                }
            }
        }

        // N4: for every 5% the share of dark modules deviates from 50%
        let dark = self.modules.iter().filter(|&&dark| dark).count();
        let percent = dark * 100 / (size * size);
        penalty + percent.abs_diff(50) / 5 * 10
    }
}

/// Runs of 5 or more of the same color (N1), and 1:1:3:1:1 patterns with 4 light modules before or after (N3)
fn line_penalty(line: &[bool]) -> usize {
    let mut penalty = 0;
    let mut run = 1;
    for i in 1..=line.len() {
        if i < line.len() && line[i] == line[i - 1] {
            run += 1;
            continue;
        }
        if run >= 5 {
            penalty += 3 + (run - 5);
        }
        run = 1;
    }

    // Treat outside the edges as light (quiet zone)
    let padded: Vec<bool> = [false; 4]
        .into_iter()
        .chain(line.iter().copied())
        .chain([false; 4])
        .collect();
    const FINDER_LIKE: [bool; 7] = [true, false, true, true, true, false, true];
    for window in padded.windows(11) {
        let light_before = window[..4].iter().all(|&dark| !dark) && window[4..] == FINDER_LIKE;
        let light_after = window[..7] == FINDER_LIKE && window[7..].iter().all(|&dark| !dark);
        penalty += 40 * (usize::from(light_before) + usize::from(light_after));
    }
    penalty
}

/// The QR code pattern
pub struct QrCode {
    matrix: Matrix,
}

impl QrCode {
    pub fn encode(text: &str) -> Result<Self, TooLong> {
        let bytes = text.as_bytes();
        let version = choose_version(bytes.len())?;
        let codewords = interleave(&encode_data(bytes, version), version);

        let mut base = Matrix::new(version);
        base.draw_function_patterns(version);
        base.draw_codewords(&codewords);

        let (_, matrix) = (0..8)
            .map(|mask| {
                let mut matrix = base.clone();
                matrix.apply_mask(mask);
                matrix.draw_format(format_bits(mask));
                (mask, matrix)
            })
            .min_by_key(|(_, matrix)| matrix.penalty())
            .expect("マスクは8種類ある");
        Ok(Self { matrix })
    }

    /// Draws into a width × height pixel buffer (one pixel per 0x00RRGGBB, row by row).
    /// Scales up by the largest integer factor that fits including the quiet zone, and centers it.
    /// The background is always white and the pattern always black, so it reads regardless of screen brightness
    pub fn render(&self, pixels: &mut [u32], width: usize, height: usize) {
        const WHITE: u32 = 0x00ff_ffff;
        const BLACK: u32 = 0x0000_0000;
        pixels.fill(WHITE);

        let modules = self.matrix.size + QUIET_ZONE * 2;
        let scale = width.min(height) / modules;
        if scale == 0 {
            return;
        }
        let left = (width - modules * scale) / 2 + QUIET_ZONE * scale;
        let top = (height - modules * scale) / 2 + QUIET_ZONE * scale;
        for y in 0..self.matrix.size * scale {
            let row = &mut pixels[(top + y) * width..][..width];
            for x in 0..self.matrix.size * scale {
                if self.matrix.dark(x / scale, y / scale) {
                    row[left + x] = BLACK;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The "HELLO WORLD" 1-M example in thonky.com's QR Code Tutorial
    #[test]
    fn reed_solomon_matches_known_example() {
        let data = [
            32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17,
        ];
        assert_eq!(
            reed_solomon(&data, 10),
            [196, 35, 39, 119, 235, 215, 231, 226, 93, 23]
        );
    }

    #[test]
    fn format_and_version_bits_match_standard_tables() {
        // Level M with mask 0 is 101010000010010; version 7 is 000111110010010100
        assert_eq!(format_bits(0), 0b101010000010010);
        assert_eq!(version_bits(7), 0b000111110010010100);
    }

    #[test]
    fn alignment_positions_match_standard_table() {
        assert!(alignment_positions(1).is_empty());
        assert_eq!(alignment_positions(2), [6, 18]);
        assert_eq!(alignment_positions(7), [6, 22, 38]);
        assert_eq!(alignment_positions(10), [6, 28, 50]);
    }

    /// Number of codewords available for data and error correction, excluding function patterns (formula from the standard)
    fn total_codeword_count(version: usize) -> usize {
        let mut modules = (16 * version + 128) * version + 64;
        if version >= 2 {
            let align = version / 7 + 2;
            modules -= (25 * align - 10) * align - 55;
            if version >= 7 {
                modules -= 36;
            }
        }
        modules / 8
    }

    #[test]
    fn block_specs_add_up_to_total_codewords() {
        for version in 1..=MAX_VERSION {
            let spec = block_spec(version);
            let blocks: usize = spec.groups.iter().map(|(n, _)| n).sum();
            let total = data_codeword_count(version) + spec.ec_per_block * blocks;
            assert_eq!(total, total_codeword_count(version), "型番 {version}");
            let matrix = {
                let mut m = Matrix::new(version);
                m.draw_function_patterns(version);
                m
            };
            assert_eq!(matrix.data_positions().len() / 8, total, "型番 {version}");
        }
    }

    /// Traces the generated pattern backwards, from reading the format information to restoring the string
    fn decode(qr: &QrCode) -> String {
        let matrix = &qr.matrix;
        let mut format = 0;
        for i in 0..=5 {
            format |= u32::from(matrix.dark(8, i)) << i;
        }
        format |= u32::from(matrix.dark(8, 7)) << 6;
        format |= u32::from(matrix.dark(8, 8)) << 7;
        format |= u32::from(matrix.dark(7, 8)) << 8;
        for i in 9..15 {
            format |= u32::from(matrix.dark(14 - i, 8)) << i;
        }
        let mask = (0..8).find(|&m| format_bits(m) == format).unwrap();
        let version = (matrix.size - 17) / 4;

        let mut unmasked = matrix.clone();
        unmasked.apply_mask(mask);
        let bits: Vec<bool> = unmasked
            .data_positions()
            .into_iter()
            .map(|(x, y)| unmasked.dark(x, y))
            .collect();
        let codewords: Vec<u8> = BitBuffer(bits).to_bytes();

        // Undo the interleaving and check the error correction codewords of each block
        let spec = block_spec(version);
        let lengths: Vec<usize> = spec
            .groups
            .iter()
            .flat_map(|&(n, k)| std::iter::repeat_n(k, n))
            .collect();
        let mut blocks = vec![Vec::new(); lengths.len()];
        let mut next = codewords.iter();
        for i in 0..*lengths.iter().max().unwrap() {
            for (b, &length) in lengths.iter().enumerate() {
                if i < length {
                    blocks[b].push(*next.next().unwrap());
                }
            }
        }
        let mut ec = vec![Vec::new(); lengths.len()];
        for _ in 0..spec.ec_per_block {
            for block in &mut ec {
                block.push(*next.next().unwrap());
            }
        }
        for (block, ec) in blocks.iter().zip(&ec) {
            assert_eq!(&reed_solomon(block, spec.ec_per_block), ec);
        }

        let data: Vec<bool> = blocks
            .concat()
            .iter()
            .flat_map(|&byte| (0..8).rev().map(move |i| (byte >> i) & 1 == 1))
            .collect();
        let read = |start: usize, length: usize| {
            data[start..start + length]
                .iter()
                .fold(0usize, |v, &bit| (v << 1) | usize::from(bit))
        };
        assert_eq!(read(0, 4), 0b0100);
        let count_length = count_bits(version);
        let count = read(4, count_length);
        let bytes: Vec<u8> = (0..count)
            .map(|i| read(4 + count_length + i * 8, 8) as u8)
            .collect();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn encoded_text_can_be_read_back() {
        let long = format!(
            "https://draw.example.com/#ip=192.168.100.200&port=47100&hash={}&token={}",
            "ab".repeat(32),
            "cd".repeat(16)
        );
        for text in [
            "a",
            "https://example.com/#ip=192.168.100.200&port=47100",
            &long,
        ] {
            let qr = QrCode::encode(text).unwrap();
            assert_eq!(decode(&qr), text);
        }
        // At the lengths real QR codes carry, it reaches version 7 and up, which includes version information
        assert!(QrCode::encode(&long).unwrap().matrix.size >= module_size(7));
    }

    #[test]
    fn has_finder_patterns_in_three_corners() {
        let qr = QrCode::encode("finder").unwrap();
        let size = qr.matrix.size;
        for (x0, y0) in [(0, 0), (size - 7, 0), (0, size - 7)] {
            for y in 0..7 {
                for x in 0..7 {
                    let ring = (x as i32 - 3).abs().max((y as i32 - 3).abs());
                    assert_eq!(qr.matrix.dark(x0 + x, y0 + y), ring != 2);
                }
            }
        }
    }

    #[test]
    fn rejects_too_long_text() {
        assert!(matches!(QrCode::encode(&"x".repeat(300)), Err(TooLong)));
    }

    #[test]
    fn renders_scaled_and_centered_with_quiet_zone() {
        let qr = QrCode::encode("a").unwrap();
        let modules = qr.matrix.size + QUIET_ZONE * 2;
        // Wide area: scaled 3x to fit the height, centered horizontally
        let (width, height) = (modules * 3 + 10, modules * 3);
        let mut pixels = vec![0; width * height];
        qr.render(&mut pixels, width, height);

        let at = |x: usize, y: usize| pixels[y * width + x];
        let origin = (5 + QUIET_ZONE * 3, QUIET_ZONE * 3);
        // The corner of the top-left finder pattern is dark, and the quiet zone is white
        assert_eq!(at(origin.0, origin.1), 0);
        assert_eq!(at(origin.0 + 2, origin.1 + 2), 0);
        assert_eq!(at(origin.0 - 1, origin.1), 0x00ff_ffff);
        assert_eq!(at(0, 0), 0x00ff_ffff);
        // 3x3 pixels per module: the second module of the finder pattern (the light ring) is white
        assert_eq!(at(origin.0 + 3, origin.1 + 3), 0x00ff_ffff);
    }

    #[test]
    fn render_leaves_blank_when_too_small() {
        let qr = QrCode::encode("a").unwrap();
        let mut pixels = vec![0; 10 * 10];
        qr.render(&mut pixels, 10, 10);
        assert!(pixels.iter().all(|&p| p == 0x00ff_ffff));
    }
}
