// GPU stream compaction of cull visibility flags into a draw list.
//
// Input is the per-cell visibility buffer produced by `cull.wgsl` (1 = keep,
// 0 = cull). This pass compacts the indices of the kept cells into a dense output
// buffer and fills the `instance_count` of an indexed indirect draw so the draw
// touches only the survivors, with no CPU readback in the steady state.
//
// Each workgroup of 256 threads:
//   1. loads one flag per thread into workgroup memory,
//   2. runs an inclusive Hillis-Steele prefix sum over the 256 flags, converted to
//      an *exclusive* prefix (the count of survivors strictly before this thread in
//      the workgroup),
//   3. has one thread reserve a contiguous output range with a single atomicAdd of
//      the workgroup's survivor total onto a global cursor, and add the same total
//      onto the indirect draw's `instance_count`,
//   4. each surviving thread writes its global instance index at
//      `base + exclusive_prefix`.
//
// The global ordering of the compacted output is unspecified (workgroups race for
// their output range), which is exactly what an instanced draw needs: it consumes the
// set of survivors, order-independent. Ordering within a workgroup is preserved by the
// scan, which keeps the writes to the reserved block disjoint and in-bounds.

struct Params {
    // Number of valid entries in `visible` (and the index space of the cells).
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

// Indexed indirect draw arguments, matching `wgpu::util::DrawIndexedIndirectArgs`.
// `instance_count` is atomic so every workgroup can add its survivor total to it.
struct DrawIndexedIndirect {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};

@group(0) @binding(0)
var<uniform> params: Params;

@group(0) @binding(1)
var<storage, read> visible: array<u32>;

@group(0) @binding(2)
var<storage, read_write> compacted: array<u32>;

// Global output cursor: workgroups atomicAdd their survivor total to reserve a range.
@group(0) @binding(3)
var<storage, read_write> cursor: atomic<u32>;

// The indirect draw arguments whose `instance_count` is accumulated here.
@group(0) @binding(4)
var<storage, read_write> draw_args: DrawIndexedIndirect;

const WORKGROUP_SIZE: u32 = 256u;

// Scratch for the prefix sum: one slot per thread. `base` carries the reserved output
// offset from the reserving thread to the whole workgroup.
var<workgroup> scan: array<u32, WORKGROUP_SIZE>;
var<workgroup> base: u32;

@compute @workgroup_size(256)
fn compact(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let index = gid.x;
    let local = lid.x;

    // Load this thread's flag (0 or 1); out-of-range threads contribute 0.
    var flag: u32 = 0u;
    if (index < params.count) {
        flag = select(0u, 1u, visible[index] != 0u);
    }
    scan[local] = flag;
    workgroupBarrier();

    // Inclusive Hillis-Steele scan over the 256 flags.
    var offset: u32 = 1u;
    for (; offset < WORKGROUP_SIZE; offset = offset << 1u) {
        var add: u32 = 0u;
        if (local >= offset) {
            add = scan[local - offset];
        }
        workgroupBarrier();
        scan[local] = scan[local] + add;
        workgroupBarrier();
    }

    // Total survivors in this workgroup is the inclusive sum at the last lane.
    let total = scan[WORKGROUP_SIZE - 1u];
    // Exclusive prefix for this thread: inclusive minus its own flag.
    let exclusive = scan[local] - flag;

    // One thread reserves the output range and bumps the draw's instance_count.
    if (local == 0u) {
        var reserved: u32 = 0u;
        if (total > 0u) {
            reserved = atomicAdd(&cursor, total);
            let _ignored = atomicAdd(&draw_args.instance_count, total);
        }
        base = reserved;
    }
    workgroupBarrier();

    // Each surviving thread writes its global index into the reserved block.
    if (flag == 1u) {
        let out = base + exclusive;
        if (out < arrayLength(&compacted)) {
            compacted[out] = index;
        }
    }
}
