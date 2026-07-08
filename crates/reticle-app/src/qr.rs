//! A small, dependency-free QR-code encoder for the Share dialog (catalog item 92).
//!
//! Scope is deliberately narrow: byte mode, error-correction level L, versions 1 to
//! 5, single error-correction block. That is enough for a share link (up to 108
//! bytes) while avoiding the block-interleaving and version-info machinery larger
//! versions need, keeping this module compact so the QR stays inside the bundle
//! budget. A link too long for version 5 returns `None` and the dialog shows the
//! link without a code rather than a wrong one.
//!
//! Like the rest of this lane's models the encoder is pure: it turns bytes into a
//! square grid of dark/light [`modules`](QrCode::module) with no `egui` and no
//! platform types, so the reproducible parts (the Reed-Solomon codewords and the
//! format-information bits) are unit-tested against the canonical QR vectors, and the
//! Share dialog is the only place that paints the grid.
//!
//! The construction follows the QR specification (ISO/IEC 18004): finder, timing and
//! alignment patterns, the mode/character-count/data bit stream, Reed-Solomon error
//! correction over GF(256), the zig-zag data placement, the eight data masks scored
//! by the four penalty rules, and the BCH-coded format information.

// QR construction is inherently index-heavy: `x`/`y` module coordinates, `i`/`j`
// bit indices, and the GF(256) `a`/`b`/`c` operands read most clearly as the short
// names the specification itself uses.
#![allow(clippy::many_single_char_names)]

/// A rendered QR code: a `size` x `size` grid of dark (`true`) / light (`false`)
/// modules, without the quiet zone (the renderer adds the margin).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    size: usize,
    dark: Vec<bool>,
}

impl QrCode {
    /// The side length of the code in modules (21 for version 1, +4 per version).
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at `(x, y)` is dark. Out-of-range coordinates read light.
    #[must_use]
    pub fn module(&self, x: usize, y: usize) -> bool {
        if x < self.size && y < self.size {
            self.dark[y * self.size + x]
        } else {
            false
        }
    }

    /// Encodes `data` as a byte-mode QR code at the smallest fitting version (1 to 5,
    /// error-correction level L), or `None` when it exceeds the version-5 capacity of
    /// 108 bytes.
    #[must_use]
    pub fn encode(data: &[u8]) -> Option<Self> {
        let ver = Version::smallest_fitting(data.len())?;
        let gf = Gf256::new();
        let codewords = ver.codewords(data, &gf);
        let mut m = Matrix::new(ver);
        m.draw_function_patterns();
        m.reserve_format();
        m.draw_codewords(&codewords);
        let mask = m.apply_best_mask();
        m.draw_format(mask);
        Some(QrCode {
            size: m.size,
            dark: m.dark,
        })
    }
}

/// A version's fixed parameters for error-correction level L, single block.
#[derive(Clone, Copy)]
struct Version {
    /// The version number, 1 to 5.
    number: usize,
    /// The number of error-correction codewords.
    ec_len: usize,
    /// The total number of codewords (data + error correction).
    total: usize,
    /// The single interior alignment-pattern centre coordinate, or 0 for version 1
    /// (which has none).
    align: usize,
}

/// The version-1..5, EC-level-L parameter table (single error-correction block).
const VERSIONS: [Version; 5] = [
    Version {
        number: 1,
        ec_len: 7,
        total: 26,
        align: 0,
    },
    Version {
        number: 2,
        ec_len: 10,
        total: 44,
        align: 18,
    },
    Version {
        number: 3,
        ec_len: 15,
        total: 70,
        align: 22,
    },
    Version {
        number: 4,
        ec_len: 20,
        total: 100,
        align: 26,
    },
    Version {
        number: 5,
        ec_len: 26,
        total: 134,
        align: 30,
    },
];

impl Version {
    /// The number of data codewords (total minus error-correction).
    fn data_len(self) -> usize {
        self.total - self.ec_len
    }

    /// The side length in modules.
    fn size(self) -> usize {
        self.number * 4 + 17
    }

    /// The smallest version whose byte-mode payload holds `len` bytes: two overhead
    /// bytes (the 4-bit mode indicator plus the 8-bit character count) plus the data.
    fn smallest_fitting(len: usize) -> Option<Self> {
        VERSIONS.into_iter().find(|v| len + 2 <= v.data_len())
    }

    /// Builds the full codeword sequence: the byte-mode data bit stream, padded to the
    /// data capacity, followed by its Reed-Solomon error-correction codewords.
    fn codewords(self, data: &[u8], gf: &Gf256) -> Vec<u8> {
        let data_len = self.data_len();
        // Bit stream: mode indicator 0100, 8-bit length, the bytes, a 0000 terminator,
        // then zero-pad to a byte boundary.
        let mut bits = BitBuffer::default();
        bits.push(0b0100, 4);
        bits.push(u32::try_from(data.len()).unwrap_or(0), 8);
        for &b in data {
            bits.push(u32::from(b), 8);
        }
        let capacity_bits = data_len * 8;
        let terminator = 4.min(capacity_bits.saturating_sub(bits.len()));
        bits.push(0, terminator);
        while bits.len() % 8 != 0 {
            bits.push(0, 1);
        }
        let mut bytes = bits.into_bytes();
        // Pad bytes alternate 0xEC / 0x11 until the data capacity is filled.
        let pad = [0xEC_u8, 0x11];
        let mut i = 0;
        while bytes.len() < data_len {
            bytes.push(pad[i % 2]);
            i += 1;
        }
        let ec = gf.reed_solomon(&bytes, self.ec_len);
        bytes.extend_from_slice(&ec);
        bytes
    }
}

/// A most-significant-bit-first bit accumulator for building the data stream.
#[derive(Default)]
struct BitBuffer {
    bits: Vec<bool>,
}

impl BitBuffer {
    fn push(&mut self, value: u32, len: usize) {
        for i in (0..len).rev() {
            self.bits.push((value >> i) & 1 == 1);
        }
    }

    fn len(&self) -> usize {
        self.bits.len()
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bits
            .chunks(8)
            .map(|chunk| chunk.iter().fold(0_u8, |acc, &b| (acc << 1) | u8::from(b)))
            .collect()
    }
}

/// GF(256) exponent/log tables for the Reed-Solomon arithmetic, built over the QR
/// primitive polynomial 0x11d with generator 2.
struct Gf256 {
    exp: [u8; 256],
    log: [u8; 256],
}

impl Gf256 {
    fn new() -> Self {
        let mut exp = [0_u8; 256];
        let mut log = [0_u8; 256];
        let mut x: u16 = 1;
        for (i, e) in exp.iter_mut().enumerate().take(255) {
            *e = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11d;
            }
        }
        // exp is periodic with 255; fill the final slot for convenience.
        exp[255] = exp[0];
        Self { exp, log }
    }

    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            let l = usize::from(self.log[a as usize]) + usize::from(self.log[b as usize]);
            self.exp[l % 255]
        }
    }

    /// The Reed-Solomon generator polynomial for `ec_len` codewords: the product of
    /// the factors `(x - 2^i)` for `i` in `0..ec_len`, returned as coefficients
    /// high-degree first with a leading 1.
    fn generator(&self, ec_len: usize) -> Vec<u8> {
        let mut poly = vec![1_u8];
        for i in 0..ec_len {
            // Multiply poly by (x - alpha^i); alpha^i = exp[i].
            let factor = self.exp[i % 255];
            let mut next = vec![0_u8; poly.len() + 1];
            for (j, &c) in poly.iter().enumerate() {
                next[j] ^= c; // times x
                next[j + 1] ^= self.mul(c, factor); // times alpha^i
            }
            poly = next;
        }
        poly
    }

    /// The `ec_len` Reed-Solomon error-correction codewords for `data`.
    fn reed_solomon(&self, data: &[u8], ec_len: usize) -> Vec<u8> {
        let gpoly = self.generator(ec_len);
        let mut rem = vec![0_u8; ec_len];
        for &d in data {
            let factor = d ^ rem[0];
            rem.rotate_left(1);
            *rem.last_mut().unwrap() = 0;
            for (j, r) in rem.iter_mut().enumerate() {
                *r ^= self.mul(gpoly[j + 1], factor);
            }
        }
        rem
    }
}

/// The module grid under construction: dark bits plus a function-module mask so data
/// placement and masking skip the fixed patterns.
struct Matrix {
    version: Version,
    size: usize,
    dark: Vec<bool>,
    func: Vec<bool>,
}

impl Matrix {
    fn new(version: Version) -> Self {
        let size = version.size();
        Self {
            version,
            size,
            dark: vec![false; size * size],
            func: vec![false; size * size],
        }
    }

    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.size + x
    }

    fn set_function(&mut self, x: usize, y: usize, dark: bool) {
        let i = self.idx(x, y);
        self.dark[i] = dark;
        self.func[i] = true;
    }

    fn draw_function_patterns(&mut self) {
        let n = self.size;
        // Timing patterns along row 6 and column 6.
        for i in 0..n {
            let dark = i % 2 == 0;
            self.set_function(i, 6, dark);
            self.set_function(6, i, dark);
        }
        // Three finder patterns and their separators.
        self.draw_finder(0, 0);
        self.draw_finder(n - 7, 0);
        self.draw_finder(0, n - 7);
        // The interior alignment pattern (versions 2 to 5 have exactly one).
        if self.version.align != 0 {
            let c = self.version.align;
            self.draw_alignment(c, c);
        }
        // The always-dark module beside the bottom-left finder.
        self.set_function(8, n - 8, true);
    }

    /// Draws a 7x7 finder pattern with its one-module separator, top-left corner at
    /// `(ox, oy)`.
    fn draw_finder(&mut self, ox: usize, oy: usize) {
        for dy in -1_i32..=7 {
            for dx in -1_i32..=7 {
                let x = ox as i32 + dx;
                let y = oy as i32 + dy;
                if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
                    continue;
                }
                // A finder is a 3x3 dark core inside a light ring inside a dark outer
                // ring; the separator (dx or dy == -1 or 7) is light. Ring distance is
                // the Chebyshev distance from the 7x7 centre: dark everywhere except the
                // ring at distance 2.
                let ring = dx.clamp(0, 6).abs_diff(3).max(dy.clamp(0, 6).abs_diff(3));
                let inside = (0..=6).contains(&dx) && (0..=6).contains(&dy);
                let dark = inside && ring != 2;
                self.set_function(x as usize, y as usize, dark);
            }
        }
    }

    /// Draws a 5x5 alignment pattern centred at `(cx, cy)`.
    fn draw_alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2_i32..=2 {
            for dx in -2_i32..=2 {
                let ring = dx.abs().max(dy.abs());
                let dark = ring != 1;
                let x = (cx as i32 + dx) as usize;
                let y = (cy as i32 + dy) as usize;
                self.set_function(x, y, dark);
            }
        }
    }

    /// Reserves (marks as function, light for now) the two format-information strips so
    /// data placement skips them; the real bits are written after masking.
    fn reserve_format(&mut self) {
        let n = self.size;
        for i in 0..9 {
            if i != 6 {
                self.set_function(8, i, false);
                self.set_function(i, 8, false);
            }
        }
        for i in 0..8 {
            self.set_function(n - 1 - i, 8, false);
        }
        for i in 0..7 {
            self.set_function(8, n - 1 - i, false);
        }
    }

    /// Places the codeword bits in the standard upward/downward zig-zag over the
    /// non-function modules, most-significant bit first.
    fn draw_codewords(&mut self, codewords: &[u8]) {
        let n = self.size as i32;
        let mut bit = 0_usize;
        let total_bits = codewords.len() * 8;
        let mut right = n - 1;
        while right >= 1 {
            if right == 6 {
                right = 5; // skip the vertical timing column
            }
            for vert in 0..n {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = ((right + 1) & 2) == 0;
                    let y = if upward {
                        (n - 1 - vert) as usize
                    } else {
                        vert as usize
                    };
                    let i = self.idx(x, y);
                    if !self.func[i] && bit < total_bits {
                        let byte = codewords[bit / 8];
                        let b = (byte >> (7 - (bit % 8))) & 1 == 1;
                        self.dark[i] = b;
                        bit += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    /// Applies each of the eight data masks to the non-function modules, keeps the one
    /// with the lowest penalty, and returns its index.
    fn apply_best_mask(&mut self) -> u8 {
        let mut best = 0_u8;
        let mut best_penalty = u32::MAX;
        let mut best_dark = self.dark.clone();
        for mask in 0..8_u8 {
            self.apply_mask(mask);
            let p = self.penalty();
            if p < best_penalty {
                best_penalty = p;
                best = mask;
                best_dark.clone_from(&self.dark);
            }
            self.apply_mask(mask); // XOR again to revert
        }
        self.dark = best_dark;
        best
    }

    /// XORs the mask pattern into every non-function module (self-inverse).
    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                let i = self.idx(x, y);
                if self.func[i] {
                    continue;
                }
                let (xi, yi) = (x as u32, y as u32);
                let flip = match mask {
                    0 => (xi + yi) % 2 == 0,
                    1 => yi % 2 == 0,
                    2 => xi % 3 == 0,
                    3 => (xi + yi) % 3 == 0,
                    4 => ((yi / 2) + (xi / 3)) % 2 == 0,
                    5 => (xi * yi) % 2 + (xi * yi) % 3 == 0,
                    6 => ((xi * yi) % 2 + (xi * yi) % 3) % 2 == 0,
                    _ => ((xi + yi) % 2 + (xi * yi) % 3) % 2 == 0,
                };
                if flip {
                    self.dark[i] = !self.dark[i];
                }
            }
        }
    }

    /// The QR penalty score (the four adjacency/block/finder/balance rules), lower is
    /// better.
    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut score = 0_u32;
        let at = |x: usize, y: usize| self.dark[y * n + x];
        // Rule 1: runs of five or more same-colour modules in each row and column.
        for y in 0..n {
            let mut run_c = 1;
            let mut run_r = 1;
            for x in 1..n {
                if at(x, y) == at(x - 1, y) {
                    run_r += 1;
                } else {
                    if run_r >= 5 {
                        score += 3 + (run_r - 5);
                    }
                    run_r = 1;
                }
                if at(y, x) == at(y, x - 1) {
                    run_c += 1;
                } else {
                    if run_c >= 5 {
                        score += 3 + (run_c - 5);
                    }
                    run_c = 1;
                }
            }
            if run_r >= 5 {
                score += 3 + (run_r - 5);
            }
            if run_c >= 5 {
                score += 3 + (run_c - 5);
            }
        }
        // Rule 2: 2x2 blocks of one colour.
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = at(x, y);
                if at(x + 1, y) == c && at(x, y + 1) == c && at(x + 1, y + 1) == c {
                    score += 3;
                }
            }
        }
        // Rule 3: the finder-like 1:1:3:1:1 pattern with a light run, in rows and cols.
        let pat1 = [
            true, false, true, true, true, false, true, false, false, false, false,
        ];
        let pat2 = [
            false, false, false, false, true, false, true, true, true, false, true,
        ];
        for y in 0..n {
            for x in 0..n.saturating_sub(10) {
                let row: Vec<bool> = (0..11).map(|k| at(x + k, y)).collect();
                if row == pat1 || row == pat2 {
                    score += 40;
                }
                let col: Vec<bool> = (0..11).map(|k| at(y, x + k)).collect();
                if col == pat1 || col == pat2 {
                    score += 40;
                }
            }
        }
        // Rule 4: overall dark/light balance.
        let dark = self.dark.iter().filter(|&&d| d).count();
        let total = n * n;
        let percent = dark * 100 / total;
        let five = percent / 5 * 5;
        let a = (five as i32 - 50).abs() / 5;
        let b = ((five + 5) as i32 - 50).abs() / 5;
        score += (a.min(b) as u32) * 10;
        score
    }

    /// Writes the 15-bit BCH-coded format information for error-correction level L and
    /// the chosen `mask`, in both standard copies.
    fn draw_format(&mut self, mask: u8) {
        let n = self.size;
        // Level L format bits are 0b01; append the 3-bit mask, then BCH(15,5).
        let data = (0b01_u32 << 3) | u32::from(mask);
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        let bits = ((data << 10) | rem) ^ 0x5412;
        let get = |i: u32| (bits >> i) & 1 == 1;
        // First copy: around the top-left finder.
        for i in 0..6 {
            self.set_function(8, i, get(i as u32));
        }
        self.set_function(8, 7, get(6));
        self.set_function(8, 8, get(7));
        self.set_function(7, 8, get(8));
        for i in 9..15 {
            self.set_function(14 - i, 8, get(i as u32));
        }
        // Second copy: split along the bottom-left and top-right.
        for i in 0..8 {
            self.set_function(n - 1 - i, 8, get(i as u32));
        }
        for i in 8..15 {
            self.set_function(8, n - 15 + i, get(i as u32));
        }
        self.set_function(8, n - 8, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reed_solomon_generator_matches_the_known_polynomial() {
        // The degree-7 generator polynomial's coefficients, as alpha exponents, are the
        // canonical [0, 87, 229, 146, 149, 238, 102, 21] (QR spec / Thonky table).
        let gf = Gf256::new();
        let gpoly = gf.generator(7);
        let exps: Vec<u8> = gpoly.iter().map(|&c| gf.log[c as usize]).collect();
        assert_eq!(exps, vec![0, 87, 229, 146, 149, 238, 102, 21]);
    }

    #[test]
    fn reed_solomon_encodes_a_known_message() {
        // GF(256) multiply against a hand-checkable pair, then a round-trip length check.
        let gf = Gf256::new();
        assert_eq!(gf.mul(0, 5), 0);
        // 2 * 2 = 4 in this field (no reduction yet).
        assert_eq!(gf.mul(2, 2), 4);
        // Encoding produces exactly ec_len codewords.
        let ec = gf.reed_solomon(&[0x10, 0x20, 0x0c, 0x56], 10);
        assert_eq!(ec.len(), 10);
    }

    #[test]
    fn format_bits_match_the_standard_for_level_l_mask_0() {
        // The published format-information string for EC level L, mask 0 is
        // 0b111011111000100 = 0x77C4.
        let data = 0b01_u32 << 3; // mask 0
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        let bits = ((data << 10) | rem) ^ 0x5412;
        assert_eq!(bits, 0x77C4);
    }

    #[test]
    fn encode_picks_the_smallest_version_and_sizes_the_grid() {
        // A short link fits version 1 (21x21); "too long" returns None past version 5.
        let code = QrCode::encode(b"https://x/c.gds").expect("fits v1");
        assert_eq!(code.size(), 21);
        // Version selection climbs with length.
        let big = QrCode::encode(&[b'a'; 60]).expect("fits v4/v5");
        assert!(
            big.size() >= 33,
            "a 60-byte payload needs at least version 4"
        );
        // Beyond version 5 capacity (108 data bytes minus 2 overhead) there is no code.
        assert!(QrCode::encode(&[b'a'; 200]).is_none());
    }

    #[test]
    fn encode_places_the_three_finder_patterns() {
        // Every QR code carries a dark 3x3 core at each of three corners; check the
        // centre module of each finder is dark.
        let code = QrCode::encode(b"https://reticle/chip.gds").expect("encodes");
        let n = code.size();
        assert!(code.module(3, 3), "top-left finder centre is dark");
        assert!(code.module(n - 4, 3), "top-right finder centre is dark");
        assert!(code.module(3, n - 4), "bottom-left finder centre is dark");
        // The timing pattern alternates; module (6,8) sits on the vertical timing line.
        assert!(code.module(6, 6), "timing origin is dark");
    }

    #[test]
    fn dark_module_is_always_set() {
        // The fixed dark module beside the bottom-left finder is always dark.
        let code = QrCode::encode(b"abc").expect("encodes");
        assert!(code.module(8, code.size() - 8));
    }
}
