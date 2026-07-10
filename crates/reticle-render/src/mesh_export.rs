//! Pure, GPU-free writers that serialize a CPU-built 3D layer-stack mesh
//! (`Mesh3d`, from [`crate::pipeline3d`]) to on-disk 3D interchange formats.
//!
//! Both writers take a mesh already built on the CPU (see `Mesh3d::build`)
//! and return bytes; they touch no GPU state, open no files, and do no I/O of
//! their own, so they run anywhere (including `wasm32`) and are unit
//! testable without a `wgpu` adapter.
//!
//! - [`to_stl_binary`] emits the binary STL format: an 80-byte header, a
//!   little-endian `u32` triangle count, then 50 bytes per triangle (a facet
//!   normal recomputed from the triangle's own vertex positions, its three
//!   vertex positions, and a zero attribute byte count). STL has no color
//!   channel, so vertex color is dropped.
//! - [`to_gltf_binary`] emits glTF 2.0 in its binary container form
//!   (`.glb`): a 12-byte header, a JSON chunk (`asset`/`scene`/`mesh`/
//!   `accessor`/`bufferView` metadata), and a `BIN` chunk holding the
//!   position, normal, color, and index data the JSON chunk's buffer views
//!   point into. The JSON is assembled with small local `json_object`/
//!   `json_array` helpers rather than a JSON crate, keeping this module free
//!   of new dependencies.

use crate::pipeline3d::Mesh3d;

// ---------------------------------------------------------------------------
// Binary STL
// ---------------------------------------------------------------------------

/// Byte length of the binary STL header record. Content is unused by
/// readers; [`STL_HEADER_TEXT`] is written there purely so a human skimming
/// the file with `strings` can see its origin.
const STL_HEADER_LEN: usize = 80;

/// Byte length of one binary STL triangle record: a `[f32; 3]` facet normal,
/// three `[f32; 3]` vertex positions, and a trailing `u16` attribute byte
/// count.
const STL_TRIANGLE_LEN: usize = (3 + 3 * 3) * 4 + 2;

/// Identifying text written into the STL header, zero-padded to
/// [`STL_HEADER_LEN`] bytes.
const STL_HEADER_TEXT: &[u8] = b"reticle-render binary STL export";

/// Serializes `mesh` to the binary STL format.
///
/// `mesh.indices` is walked in chunks of 3 (one per triangle); a trailing
/// partial triangle, or a triangle referencing an index past the end of
/// `mesh.vertices`, is dropped rather than causing a panic. The facet normal
/// is recomputed from each triangle's own vertex positions (the normalized
/// cross product of its two edges) rather than reused from the vertex data,
/// so the output is correct even when the source mesh carries smoothed
/// per-vertex normals; a degenerate (zero-area) triangle writes a zero
/// normal, which is valid STL (readers that care recompute it themselves).
#[must_use]
pub fn to_stl_binary(mesh: &Mesh3d) -> Vec<u8> {
    let triangle_count = mesh.indices.len() / 3;
    let mut out = Vec::with_capacity(STL_HEADER_LEN + 4 + triangle_count * STL_TRIANGLE_LEN);

    let mut header = [0u8; STL_HEADER_LEN];
    header[..STL_HEADER_TEXT.len()].copy_from_slice(STL_HEADER_TEXT);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangle_count as u32).to_le_bytes());

    for tri in mesh.indices.chunks_exact(3) {
        let found = (
            mesh.vertices.get(tri[0] as usize),
            mesh.vertices.get(tri[1] as usize),
            mesh.vertices.get(tri[2] as usize),
        );
        let (Some(a), Some(b), Some(c)) = found else {
            continue;
        };
        write_stl_triangle(&mut out, a.position, b.position, c.position);
    }

    out
}

/// Appends one 50-byte binary STL triangle record (facet normal, the three
/// vertex positions in order, then a zero attribute byte count) to `out`.
fn write_stl_triangle(out: &mut Vec<u8>, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
    for component in facet_normal(a, b, c) {
        out.extend_from_slice(&component.to_le_bytes());
    }
    for vertex in [a, b, c] {
        for component in vertex {
            out.extend_from_slice(&component.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u16.to_le_bytes());
}

/// The outward facet normal of triangle `(a, b, c)`: the normalized cross
/// product of its two edges. Returns `[0.0, 0.0, 0.0]` for a degenerate
/// (zero-area) triangle instead of dividing by zero.
fn facet_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let ab = sub3(b, a);
    let ac = sub3(c, a);
    let raw = cross3(ab, ac);
    let len = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
    if len > f32::EPSILON {
        [raw[0] / len, raw[1] / len, raw[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// `a - b`, componentwise.
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The 3D cross product `a x b`.
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

// ---------------------------------------------------------------------------
// Binary glTF (.glb)
// ---------------------------------------------------------------------------

/// GLB magic bytes: the ASCII bytes `b"glTF"` read as a little-endian `u32`.
const GLB_MAGIC: u32 = 0x4654_6C67;
/// The GLB container format version this writer emits.
const GLB_VERSION: u32 = 2;
/// GLB chunk type tag for the JSON chunk: the ASCII bytes `b"JSON"` read as a
/// little-endian `u32`.
const GLB_CHUNK_TYPE_JSON: u32 = 0x4E4F_534A;
/// GLB chunk type tag for the binary buffer chunk: the ASCII bytes `b"BIN\0"`
/// read as a little-endian `u32`.
const GLB_CHUNK_TYPE_BIN: u32 = 0x004E_4942;

/// glTF accessor `componentType` for a 32-bit IEEE float.
const COMPONENT_TYPE_FLOAT: u32 = 5_126;
/// glTF accessor `componentType` for a 32-bit unsigned integer (indices).
const COMPONENT_TYPE_UNSIGNED_INT: u32 = 5_125;
/// glTF `bufferView.target` for vertex attribute data.
const TARGET_ARRAY_BUFFER: u32 = 34_962;
/// glTF `bufferView.target` for index data.
const TARGET_ELEMENT_ARRAY_BUFFER: u32 = 34_963;
/// glTF primitive `mode` for a triangle list, matching `Mesh3d::indices`.
const PRIMITIVE_MODE_TRIANGLES: u32 = 4;

/// The byte layout of the four attribute/index blocks packed into a binary
/// glTF's `BIN` chunk: positions, normals, and colors are each one
/// contiguous run of `f32` components per vertex (`VEC3`, `VEC3`, `VEC4`),
/// followed by the `u32` index stream. Computed once and shared between the
/// binary packer and the JSON `bufferViews` describing it, so the two can
/// never disagree.
struct GltfLayout {
    position_len: usize,
    normal_len: usize,
    color_len: usize,
    index_len: usize,
    normal_offset: usize,
    color_offset: usize,
    index_offset: usize,
    total_len: usize,
}

impl GltfLayout {
    fn new(vertex_count: usize, index_count: usize) -> Self {
        let position_len = vertex_count * 3 * 4;
        let normal_len = vertex_count * 3 * 4;
        let color_len = vertex_count * 4 * 4;
        let index_len = index_count * 4;
        let normal_offset = position_len;
        let color_offset = normal_offset + normal_len;
        let index_offset = color_offset + color_len;
        let total_len = index_offset + index_len;
        Self {
            position_len,
            normal_len,
            color_len,
            index_len,
            normal_offset,
            color_offset,
            index_offset,
            total_len,
        }
    }
}

/// Serializes `mesh` to binary glTF 2.0 (`.glb`): a 12-byte header, a JSON
/// chunk describing one mesh with one triangle-list primitive over four
/// accessors (`POSITION`, `NORMAL`, `COLOR_0`, indices), and a `BIN` chunk
/// holding that data. `mesh.indices` is written verbatim as the `u32` index
/// accessor; glTF 2.0 permits `UNSIGNED_INT` indices.
///
/// An empty `mesh` still produces a structurally valid (if degenerate) file:
/// its accessors report zero counts and its bounding box collapses to the
/// origin.
#[must_use]
pub fn to_gltf_binary(mesh: &Mesh3d) -> Vec<u8> {
    let layout = GltfLayout::new(mesh.vertices.len(), mesh.indices.len());
    let bin = pack_gltf_buffer(mesh, &layout);
    let (min, max) = mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]));
    let json = gltf_json(mesh, &layout, min, max);
    pack_glb(json.as_bytes(), &bin)
}

/// Packs `mesh`'s vertex and index data into the `BIN` chunk byte layout
/// `layout` describes: positions, then normals, then colors, then indices,
/// each a little-endian dump of the source `f32`/`u32` components.
fn pack_gltf_buffer(mesh: &Mesh3d, layout: &GltfLayout) -> Vec<u8> {
    let mut bin = Vec::with_capacity(layout.total_len);
    for vertex in &mesh.vertices {
        for component in vertex.position {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    for vertex in &mesh.vertices {
        for component in vertex.normal {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    for vertex in &mesh.vertices {
        for component in vertex.color {
            bin.extend_from_slice(&component.to_le_bytes());
        }
    }
    for index in &mesh.indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }
    debug_assert_eq!(
        bin.len(),
        layout.total_len,
        "GltfLayout disagrees with the bytes packed"
    );
    bin
}

/// Joins `items`, each already valid JSON text, into a JSON array literal.
fn json_array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

/// Joins `pairs` into a JSON object literal. Each value must already be
/// valid JSON text (a quoted string, a number, or a nested object/array);
/// building the document this way, rather than hand-writing escaped braces,
/// keeps every object and array balanced by construction.
fn json_object(pairs: &[(&str, String)]) -> String {
    let body: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("\"{key}\":{value}"))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// Formats `v` as a quoted JSON string literal. Only used for the small
/// fixed set of ASCII identifiers this writer itself emits (glTF type and
/// attribute names), so no escaping is implemented.
fn json_str(v: &str) -> String {
    format!("\"{v}\"")
}

/// Builds the JSON chunk text for `mesh`: `asset`, a one-node `scene`
/// pointing at a one-primitive `mesh`, four `accessors` (`POSITION` carries
/// the required `min`/`max`), the `bufferViews` matching `layout`, and a
/// single GLB-embedded `buffers` entry (no `uri`, per the glTF binary spec:
/// the buffer's bytes live in the container's own `BIN` chunk instead).
fn gltf_json(mesh: &Mesh3d, layout: &GltfLayout, min: [f32; 3], max: [f32; 3]) -> String {
    let vertex_count = mesh.vertices.len();
    let index_count = mesh.indices.len();

    let asset = json_object(&[
        ("version", json_str("2.0")),
        ("generator", json_str("reticle-render mesh_export")),
    ]);

    let attributes = json_object(&[
        ("POSITION", (0).to_string()),
        ("NORMAL", (1).to_string()),
        ("COLOR_0", (2).to_string()),
    ]);
    let primitive = json_object(&[
        ("attributes", attributes),
        ("indices", (3).to_string()),
        ("mode", PRIMITIVE_MODE_TRIANGLES.to_string()),
    ]);
    let meshes = json_array(&[json_object(&[("primitives", json_array(&[primitive]))])]);

    let vec3_json =
        |v: [f32; 3]| json_array(&[v[0].to_string(), v[1].to_string(), v[2].to_string()]);
    let position_accessor = json_object(&[
        ("bufferView", (0).to_string()),
        ("componentType", COMPONENT_TYPE_FLOAT.to_string()),
        ("count", vertex_count.to_string()),
        ("type", json_str("VEC3")),
        ("min", vec3_json(min)),
        ("max", vec3_json(max)),
    ]);
    let normal_accessor = json_object(&[
        ("bufferView", (1).to_string()),
        ("componentType", COMPONENT_TYPE_FLOAT.to_string()),
        ("count", vertex_count.to_string()),
        ("type", json_str("VEC3")),
    ]);
    let color_accessor = json_object(&[
        ("bufferView", (2).to_string()),
        ("componentType", COMPONENT_TYPE_FLOAT.to_string()),
        ("count", vertex_count.to_string()),
        ("type", json_str("VEC4")),
    ]);
    let index_accessor = json_object(&[
        ("bufferView", (3).to_string()),
        ("componentType", COMPONENT_TYPE_UNSIGNED_INT.to_string()),
        ("count", index_count.to_string()),
        ("type", json_str("SCALAR")),
    ]);
    let accessors = json_array(&[
        position_accessor,
        normal_accessor,
        color_accessor,
        index_accessor,
    ]);

    let buffer_view = |offset: usize, len: usize, target: u32| {
        json_object(&[
            ("buffer", (0).to_string()),
            ("byteOffset", offset.to_string()),
            ("byteLength", len.to_string()),
            ("target", target.to_string()),
        ])
    };
    let buffer_views = json_array(&[
        buffer_view(0, layout.position_len, TARGET_ARRAY_BUFFER),
        buffer_view(layout.normal_offset, layout.normal_len, TARGET_ARRAY_BUFFER),
        buffer_view(layout.color_offset, layout.color_len, TARGET_ARRAY_BUFFER),
        buffer_view(
            layout.index_offset,
            layout.index_len,
            TARGET_ELEMENT_ARRAY_BUFFER,
        ),
    ]);

    let buffers = json_array(&[json_object(&[("byteLength", layout.total_len.to_string())])]);

    json_object(&[
        ("asset", asset),
        ("scene", (0).to_string()),
        (
            "scenes",
            json_array(&[json_object(&[("nodes", json_array(&[(0).to_string()]))])]),
        ),
        (
            "nodes",
            json_array(&[json_object(&[("mesh", (0).to_string())])]),
        ),
        ("meshes", meshes),
        ("accessors", accessors),
        ("bufferViews", buffer_views),
        ("buffers", buffers),
    ])
}

/// Assembles a binary glTF (`.glb`) container from a JSON chunk and a `BIN`
/// chunk: a 12-byte header (magic, version, total length), then each chunk
/// as a `u32` length + `u32` type + payload, padded to a 4-byte boundary
/// with spaces (JSON, per spec) or zero bytes (`BIN`, per spec).
fn pack_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let json_padded_len = round_up_4(json.len());
    let bin_padded_len = round_up_4(bin.len());
    let total_len = 12 + (8 + json_padded_len) + (8 + bin_padded_len);

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&GLB_VERSION.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());

    out.extend_from_slice(&(json_padded_len as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_TYPE_JSON.to_le_bytes());
    out.extend_from_slice(json);
    out.resize(out.len() + (json_padded_len - json.len()), b' ');

    out.extend_from_slice(&(bin_padded_len as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_TYPE_BIN.to_le_bytes());
    out.extend_from_slice(bin);
    out.resize(out.len() + (bin_padded_len - bin.len()), 0);

    out
}

/// Rounds `n` up to the next multiple of 4, the GLB chunk alignment.
fn round_up_4(n: usize) -> usize {
    n.div_ceil(4) * 4
}
