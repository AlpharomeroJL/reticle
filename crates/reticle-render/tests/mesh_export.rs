//! GPU-free tests for `reticle_render::mesh_export`: the binary STL and
//! binary glTF (`.glb`) writers over a small hand-built fixture mesh. No
//! `wgpu` adapter is created anywhere in this file.

use reticle_render::{Mesh3d, Vertex3d, to_gltf_binary, to_stl_binary};

/// A two-triangle quad in the `z = 0` plane: 4 vertices, 6 indices (2
/// triangles), wound CCW as seen from `+z` (matching `Mesh3d::build`'s top
/// cap convention) so the STL facet normal comes out `(0, 0, 1)`.
fn fixture_mesh() -> Mesh3d {
    let vertex = |x: f32, y: f32| Vertex3d {
        position: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        color: [1.0, 0.0, 0.0, 1.0],
    };
    Mesh3d {
        vertices: vec![
            vertex(0.0, 0.0),
            vertex(1.0, 0.0),
            vertex(1.0, 1.0),
            vertex(0.0, 1.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

/// A small, dependency-free FNV-1a 64-bit hash, used only to pin a
/// byte-exact golden for the STL writer without adding a checksum crate.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

// The fixture's edge vectors are small exact integers, so the cross product
// and normalize below land on a bit-exact (0, 0, 1) rather than an
// approximation; comparing it exactly is intentional, not a rounding bug.
#[allow(clippy::float_cmp)]
#[test]
fn mesh_export_stl_binary_triangle_count_and_checksum() {
    let mesh = fixture_mesh();
    let stl = to_stl_binary(&mesh);

    // 80-byte header + 4-byte count + 2 triangles * 50 bytes each.
    assert_eq!(stl.len(), 80 + 4 + 2 * 50);

    let triangle_count = u32::from_le_bytes(stl[80..84].try_into().unwrap());
    assert_eq!(triangle_count, 2);

    // Both triangles are coplanar in z = 0 with CCW winding seen from +z, so
    // every facet normal should come out (0, 0, 1) regardless of the source
    // vertex normals (the writer recomputes it from triangle geometry).
    let normal0: [f32; 3] = [
        f32::from_le_bytes(stl[84..88].try_into().unwrap()),
        f32::from_le_bytes(stl[88..92].try_into().unwrap()),
        f32::from_le_bytes(stl[92..96].try_into().unwrap()),
    ];
    assert_eq!(normal0, [0.0, 0.0, 1.0]);

    // Byte-exact golden: pins the header text, field order, and endianness.
    // If the writer's output intentionally changes shape, recompute this
    // with `fnv1a64` over the new bytes rather than guessing.
    assert_eq!(
        fnv1a64(&stl),
        0xd937_db7d_d0ae_2ede,
        "recompute the golden if the STL layout changes on purpose"
    );
}

#[test]
fn mesh_export_stl_binary_drops_trailing_partial_triangle() {
    let mut mesh = fixture_mesh();
    mesh.indices.push(0); // one extra, dangling index: 7 total, not a multiple of 3
    let stl = to_stl_binary(&mesh);
    // The dangling index does not start a new triangle.
    assert_eq!(stl.len(), 80 + 4 + 2 * 50);
}

// ---------------------------------------------------------------------------
// A minimal JSON parser, just enough to validate the glTF JSON chunk below
// without adding a JSON crate dependency to this GPU-heavy crate.
// ---------------------------------------------------------------------------

/// A parsed JSON value.
#[derive(Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, want: u8) -> Result<(), String> {
        match self.bump() {
            Some(b) if b == want => Ok(()),
            other => Err(format!(
                "expected {:?} at byte {}, got {other:?}",
                want as char, self.pos
            )),
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(Json::Str(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", Json::Bool(true)),
            Some(b'f') => self.parse_literal("false", Json::Bool(false)),
            Some(b'n') => self.parse_literal("null", Json::Null),
            Some(b) if b == b'-' || b.is_ascii_digit() => self.parse_number(),
            other => Err(format!("unexpected byte {other:?} at {}", self.pos)),
        }
    }

    fn parse_literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        for expected in text.bytes() {
            self.expect(expected)?;
        }
        Ok(value)
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b'}') => break,
                other => return Err(format!("expected ',' or '}}', got {other:?}")),
            }
        }
        Ok(Json::Obj(entries))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => {}
                Some(b']') => break,
                other => return Err(format!("expected ',' or ']', got {other:?}")),
            }
        }
        Ok(Json::Arr(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut s = String::new();
        loop {
            match self.bump() {
                Some(b'"') => break,
                Some(b'\\') => {
                    let escaped = self.bump().ok_or("unterminated escape")?;
                    s.push(escaped as char);
                }
                Some(b) => s.push(b as char),
                None => return Err("unterminated string".to_string()),
            }
        }
        Ok(s)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b) if b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|e| e.to_string())
    }
}

/// Parses `text` as JSON, erroring on malformed input rather than panicking.
fn parse_json(text: &str) -> Result<Json, String> {
    let mut parser = JsonParser::new(text);
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(format!("trailing data at byte {}", parser.pos));
    }
    Ok(value)
}

#[test]
fn mesh_export_gltf_binary_is_a_valid_glb_with_expected_counts() {
    let mesh = fixture_mesh();
    let glb = to_gltf_binary(&mesh);

    // 12-byte GLB header: magic, version, total length.
    assert_eq!(&glb[0..4], b"glTF");
    let version = u32::from_le_bytes(glb[4..8].try_into().unwrap());
    assert_eq!(version, 2);
    let total_len = u32::from_le_bytes(glb[8..12].try_into().unwrap());
    assert_eq!(total_len as usize, glb.len());

    // JSON chunk header, then the JSON payload itself.
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    let json_bytes = &glb[20..20 + json_len];
    let json_text = std::str::from_utf8(json_bytes).expect("JSON chunk must be UTF-8");

    let value = parse_json(json_text).expect("emitted glTF JSON chunk must parse as JSON");

    let accessors = value
        .get("accessors")
        .and_then(Json::as_array)
        .expect("accessors array");
    assert_eq!(accessors.len(), 4, "POSITION, NORMAL, COLOR_0, indices");

    let meshes = value
        .get("meshes")
        .and_then(Json::as_array)
        .expect("meshes array");
    assert_eq!(meshes.len(), 1);
    let primitives = meshes[0]
        .get("primitives")
        .and_then(Json::as_array)
        .expect("primitives array");
    assert_eq!(primitives.len(), 1);

    // Fixture has 4 vertices (POSITION/NORMAL/COLOR_0 accessors) and 6
    // indices (2 triangles).
    let position_count = accessors[0].get("count").and_then(Json::as_f64).unwrap();
    assert_eq!(position_count as u64, 4);
    let index_count = accessors[3].get("count").and_then(Json::as_f64).unwrap();
    assert_eq!(index_count as u64, 6);

    // BIN chunk header follows the space-padded JSON chunk, and it should
    // reach exactly the end of the file.
    let bin_start = 20 + json_len;
    let bin_len = u32::from_le_bytes(glb[bin_start..bin_start + 4].try_into().unwrap()) as usize;
    assert_eq!(&glb[bin_start + 4..bin_start + 8], b"BIN\0");
    assert_eq!(bin_start + 8 + bin_len, glb.len());
}

#[test]
fn mesh_export_gltf_binary_of_empty_mesh_is_still_valid_json() {
    let glb = to_gltf_binary(&Mesh3d::default());
    assert_eq!(&glb[0..4], b"glTF");
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json_text = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
    let value = parse_json(json_text).expect("empty mesh must still emit valid JSON");
    let accessors = value.get("accessors").and_then(Json::as_array).unwrap();
    let position_count = accessors[0].get("count").and_then(Json::as_f64).unwrap();
    assert_eq!(position_count as u64, 0);
}
