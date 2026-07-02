//! Fixed-size GPU buffer pages with a free-list suballocator.
//!
//! The offscreen path allocates a fresh `create_buffer_init` for every geometry
//! buffer each frame; at 10M shapes that is hundreds of megabytes reallocated per
//! frame and forbids incremental updates. This module replaces it with a bank of
//! fixed-size GPU *pages* (a few MiB each) plus a byte-granular free-list
//! allocator, so:
//!
//! * A chunk's geometry is suballocated into a page once and updated in place via
//!   `queue.write_buffer`; no per-frame buffer creation in steady state.
//! * Freed regions (from a retessellated or removed chunk) go back on the free list
//!   and are reused, so churn does not grow memory without bound.
//! * When no page has room, one more page is added rather than reallocating a single
//!   ever-growing buffer, so there is never one 256 MiB monolith.
//!
//! [`PageAllocator`] is the pure bookkeeping (no GPU), unit-tested here.
//! [`BufferPages`] wraps it around real `wgpu` buffers and the `write_buffer`
//! uploads.

use std::collections::HashMap;

/// A default page size: 8 MiB. Large enough that a page holds many chunks, small
/// enough that growth is fine-grained and a page fits comfortably within device
/// `max_buffer_size`. Callers can pick another size via [`PageAllocator::new`].
pub const DEFAULT_PAGE_SIZE: u64 = 8 * 1024 * 1024;

/// A suballocation handed out by [`PageAllocator`]: which page, the byte offset
/// within it, and the length in bytes. Opaque to callers except for
/// [`Allocation::page`], [`Allocation::offset`], and [`Allocation::len`], which the
/// GPU layer needs to target the right `write_buffer`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Allocation {
    /// Index of the page this allocation lives in.
    page: usize,
    /// Byte offset of the allocation within its page.
    offset: u64,
    /// Length of the allocation in bytes (the requested size, before alignment
    /// padding; the padded span is tracked internally for reuse).
    len: u64,
    /// The aligned span actually reserved (>= `len`), returned to the free list on
    /// [`PageAllocator::free`]. Kept here so `free` needs only the handle.
    span: u64,
}

impl Allocation {
    /// The page index this allocation targets.
    #[must_use]
    pub fn page(&self) -> usize {
        self.page
    }

    /// The byte offset within the page.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// The usable length in bytes.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether the allocation is zero-length.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A maximal run of free bytes within a page: `[start, start + len)`.
#[derive(Clone, Copy, Debug)]
struct FreeSpan {
    start: u64,
    len: u64,
}

/// One page's free list: the free spans within it, kept sorted by `start` and
/// coalesced so adjacent frees merge back into one span.
#[derive(Clone, Debug)]
struct Page {
    /// Total page size in bytes.
    size: u64,
    /// Free spans, disjoint, sorted ascending by `start`, none adjacent.
    free: Vec<FreeSpan>,
}

impl Page {
    fn new(size: u64) -> Self {
        Self {
            size,
            free: vec![FreeSpan {
                start: 0,
                len: size,
            }],
        }
    }

    /// Total free bytes in this page (sum of spans, ignoring fragmentation).
    fn free_bytes(&self) -> u64 {
        self.free.iter().map(|s| s.len).sum()
    }

    /// First-fit allocate `span` aligned bytes, returning the offset if a span had
    /// room. Splits or consumes the chosen free span.
    fn alloc(&mut self, span: u64) -> Option<u64> {
        let idx = self.free.iter().position(|s| s.len >= span)?;
        let chosen = self.free[idx];
        let offset = chosen.start;
        if chosen.len == span {
            self.free.remove(idx);
        } else {
            self.free[idx] = FreeSpan {
                start: chosen.start + span,
                len: chosen.len - span,
            };
        }
        Some(offset)
    }

    /// Return `[start, start + span)` to the free list, coalescing with any spans
    /// immediately before or after it.
    fn free_region(&mut self, start: u64, span: u64) {
        // Insert in sorted order, then merge with the previous span (if adjacent)
        // and the next span (if adjacent). Two merges suffice to keep the list
        // maximal because it was maximal before the insert.
        let pos = self.free.partition_point(|s| s.start < start);
        self.free.insert(pos, FreeSpan { start, len: span });
        self.coalesce_forward(pos); // merge inserted span with its successor
        if pos > 0 {
            self.coalesce_forward(pos - 1); // merge predecessor with inserted span
        }
    }

    /// Merge the span at `idx` with the following span if they are adjacent.
    fn coalesce_forward(&mut self, idx: usize) {
        if idx + 1 < self.free.len() {
            let cur = self.free[idx];
            let next = self.free[idx + 1];
            if cur.start + cur.len == next.start {
                self.free[idx].len += next.len;
                self.free.remove(idx + 1);
            }
        }
    }
}

/// Byte-granular suballocator over a growing bank of fixed-size pages.
///
/// Allocations are aligned up to [`PageAllocator::alignment`] so each region can
/// legally back a `wgpu` binding. Requests larger than one page fail (return
/// `None`); the GPU layer sizes pages so that never happens for its chunks.
#[derive(Clone, Debug)]
pub struct PageAllocator {
    page_size: u64,
    alignment: u64,
    pages: Vec<Page>,
    /// Live allocations, so a handle can be validated on free and the count queried
    /// in tests. Keyed by `(page, offset)`, valued by the reserved span.
    live: HashMap<(usize, u64), u64>,
}

impl PageAllocator {
    /// Creates an allocator with the given page size and allocation alignment
    /// (both clamped to at least 1; `alignment` is rounded to a sensible minimum of
    /// 4 bytes so vertex data stays aligned).
    #[must_use]
    pub fn new(page_size: u64, alignment: u64) -> Self {
        Self {
            page_size: page_size.max(1),
            alignment: alignment.max(4),
            pages: Vec::new(),
            live: HashMap::new(),
        }
    }

    /// The size of each page in bytes.
    #[must_use]
    pub fn page_size(&self) -> u64 {
        self.page_size
    }

    /// The allocation alignment in bytes.
    #[must_use]
    pub fn alignment(&self) -> u64 {
        self.alignment
    }

    /// The number of pages currently backing the allocator.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The number of live (allocated, not yet freed) regions.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Total free bytes across all pages (does not account for fragmentation).
    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.pages.iter().map(Page::free_bytes).sum()
    }

    /// Rounds `size` up to the allocation alignment.
    fn aligned(&self, size: u64) -> u64 {
        size.div_ceil(self.alignment) * self.alignment
    }

    /// Allocates `size` bytes, returning `None` only if `size` exceeds one page.
    ///
    /// Tries every existing page first-fit; if none has room, adds a page and
    /// allocates from it. A zero-size request yields a valid empty allocation that
    /// reserves the alignment quantum (so distinct empty allocations stay distinct).
    pub fn alloc(&mut self, size: u64) -> Option<Allocation> {
        let span = self.aligned(size).max(self.alignment);
        if span > self.page_size {
            return None; // a single allocation cannot exceed a page
        }
        // First-fit across existing pages.
        for page_idx in 0..self.pages.len() {
            if let Some(offset) = self.pages[page_idx].alloc(span) {
                return Some(self.record(page_idx, offset, size, span));
            }
        }
        // No room anywhere: grow by one page and allocate from it.
        let page_idx = self.pages.len();
        let mut page = Page::new(self.page_size);
        let offset = page.alloc(span)?;
        self.pages.push(page);
        Some(self.record(page_idx, offset, size, span))
    }

    /// Records a fresh allocation in the live table and builds its handle.
    fn record(&mut self, page: usize, offset: u64, len: u64, span: u64) -> Allocation {
        self.live.insert((page, offset), span);
        Allocation {
            page,
            offset,
            len,
            span,
        }
    }

    /// Frees a previously returned allocation, returning its reserved span to the
    /// page's free list for reuse. Double frees and stale handles are ignored.
    pub fn free(&mut self, alloc: Allocation) {
        if self.live.remove(&(alloc.page, alloc.offset)).is_none() {
            return; // not live: double free or foreign handle
        }
        if let Some(page) = self.pages.get_mut(alloc.page) {
            page.free_region(alloc.offset, alloc.span);
        }
    }

    /// Frees everything, resetting every page to fully free without dropping the
    /// pages themselves (so their GPU buffers can be reused).
    pub fn reset(&mut self) {
        self.live.clear();
        for page in &mut self.pages {
            page.free = vec![FreeSpan {
                start: 0,
                len: page.size,
            }];
        }
    }
}

/// A bank of fixed-size `wgpu` buffers driven by a [`PageAllocator`].
///
/// Each page is one GPU buffer of [`PageAllocator::page_size`] bytes. Allocating a
/// chunk reserves a byte range (growing the bank by a buffer if needed) and uploads
/// its bytes with `queue.write_buffer`; freeing a chunk returns the range for reuse.
/// No buffer is recreated in steady state, and there is never a single monolithic
/// buffer: memory grows one page at a time.
///
/// Pages carry `COPY_DST` so `write_buffer` can target them, plus the caller's usage
/// (typically `VERTEX` or `INDEX`).
pub struct BufferPages {
    allocator: PageAllocator,
    buffers: Vec<wgpu::Buffer>,
    /// The full usage the GPU buffers are created with (`base_usage | COPY_DST`).
    usage: wgpu::BufferUsages,
    /// The caller's usage without the implicit `COPY_DST`, preserved so a
    /// [`BufferPages::with_page_size`] rebuild keeps the same buffer kind.
    base_usage: wgpu::BufferUsages,
    label: &'static str,
}

impl core::fmt::Debug for BufferPages {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BufferPages")
            .field("pages", &self.buffers.len())
            .field("page_size", &self.allocator.page_size())
            .field("live", &self.allocator.live_count())
            .finish_non_exhaustive()
    }
}

impl BufferPages {
    /// Creates an empty bank whose pages are `page_size` bytes each and carry
    /// `usage | COPY_DST`. Allocations are aligned to the device's
    /// `min_uniform_buffer_offset_alignment`-friendly 256 bytes so any range can be
    /// bound or copied to legally.
    #[must_use]
    pub fn new(page_size: u64, usage: wgpu::BufferUsages, label: &'static str) -> Self {
        // 256-byte alignment satisfies wgpu copy/binding offset rules on all targets.
        let allocator = PageAllocator::new(page_size, wgpu::COPY_BUFFER_ALIGNMENT.max(256));
        Self {
            allocator,
            buffers: Vec::new(),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            base_usage: usage,
            label,
        }
    }

    /// The number of GPU page buffers currently allocated.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.buffers.len()
    }

    /// The GPU buffer for a page index, if it exists.
    #[must_use]
    pub fn page_buffer(&self, page: usize) -> Option<&wgpu::Buffer> {
        self.buffers.get(page)
    }

    /// The byte size of each page.
    #[must_use]
    pub fn page_size(&self) -> u64 {
        self.allocator.page_size()
    }

    /// A fresh, empty bank with the same usage and label but a larger page size.
    ///
    /// Used when a single upload outgrows the current page size: the caller swaps in
    /// a bigger-paged bank so the whole buffer still fits one page (and one draw).
    /// This drops the old GPU buffers, so it must only be done on a structural change,
    /// never on a camera move.
    #[must_use]
    pub fn with_page_size(&self, page_size: u64) -> Self {
        Self::new(page_size, self.base_usage, self.label)
    }

    /// Reserves space for `bytes` and uploads them, returning the allocation.
    ///
    /// Grows the bank by one GPU buffer if no page had room. Returns `None` only if
    /// `bytes` is larger than a whole page. Empty input reserves a minimal aligned
    /// slot and uploads nothing.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
    ) -> Option<Allocation> {
        let alloc = self.allocator.alloc(bytes.len() as u64)?;
        // The allocator may have added a page; ensure a matching GPU buffer exists.
        self.ensure_page_buffer(device, alloc.page());
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffers[alloc.page()], alloc.offset(), bytes);
        }
        Some(alloc)
    }

    /// Overwrites an existing allocation's bytes in place via `write_buffer`.
    ///
    /// The caller must pass no more bytes than the allocation's length; extra bytes
    /// are ignored so a stray write cannot spill into a neighbor.
    pub fn write(&self, queue: &wgpu::Queue, alloc: &Allocation, bytes: &[u8]) {
        let Some(buffer) = self.buffers.get(alloc.page()) else {
            return;
        };
        let n = bytes.len().min(alloc.len() as usize);
        if n > 0 {
            queue.write_buffer(buffer, alloc.offset(), &bytes[..n]);
        }
    }

    /// Returns an allocation's range to the free list for reuse. The GPU bytes are
    /// left as-is (a later `upload` overwrites them); only bookkeeping changes.
    pub fn free(&mut self, alloc: Allocation) {
        self.allocator.free(alloc);
    }

    /// Frees every allocation, keeping the page buffers for reuse.
    pub fn reset(&mut self) {
        self.allocator.reset();
    }

    /// Creates the GPU buffer for `page` if it does not exist yet. Pages are added
    /// one at a time, so this creates at most one buffer per call.
    fn ensure_page_buffer(&mut self, device: &wgpu::Device, page: usize) {
        while self.buffers.len() <= page {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.allocator.page_size(),
                usage: self.usage,
                mapped_at_creation: false,
            });
            self.buffers.push(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Allocation, PageAllocator};

    /// A small page size keeps the arithmetic in the tests easy to read.
    const PAGE: u64 = 1024;
    const ALIGN: u64 = 16;

    fn allocator() -> PageAllocator {
        PageAllocator::new(PAGE, ALIGN)
    }

    #[test]
    fn alloc_aligns_and_stays_in_one_page() {
        let mut a = allocator();
        let x = a.alloc(10).unwrap(); // rounds up to 16
        let y = a.alloc(1).unwrap(); // rounds up to 16, next slot
        assert_eq!(x.offset(), 0);
        assert_eq!(x.len(), 10);
        assert_eq!(y.offset(), 16);
        assert_eq!(a.page_count(), 1);
        assert_eq!(a.live_count(), 2);
        // Offsets are aligned.
        assert_eq!(x.offset() % ALIGN, 0);
        assert_eq!(y.offset() % ALIGN, 0);
    }

    #[test]
    fn free_returns_space_and_reuse_fills_the_hole() {
        let mut a = allocator();
        let x = a.alloc(16).unwrap();
        let y = a.alloc(16).unwrap();
        let z = a.alloc(16).unwrap();
        assert_eq!((x.offset(), y.offset(), z.offset()), (0, 16, 32));
        assert_eq!(a.live_count(), 3);

        // Free the middle allocation; a same-size request must reuse that exact hole.
        a.free(y);
        assert_eq!(a.live_count(), 2);
        let reused = a.alloc(16).unwrap();
        assert_eq!(reused.offset(), 16, "reuse should fill the freed hole");
        assert_eq!(a.page_count(), 1, "reuse must not grow a new page");
    }

    #[test]
    fn adjacent_frees_coalesce() {
        let mut a = allocator();
        let x = a.alloc(16).unwrap();
        let y = a.alloc(16).unwrap();
        let z = a.alloc(16).unwrap();
        // Free x and y (adjacent). They must coalesce into a 32-byte span so a
        // single 32-byte allocation fits at offset 0.
        a.free(x);
        a.free(y);
        let big = a.alloc(32).unwrap();
        assert_eq!(big.offset(), 0, "coalesced span should back the 32B alloc");
        // z is still live and untouched.
        assert_eq!(z.offset(), 32);
        assert_eq!(a.live_count(), 2);
    }

    #[test]
    fn grows_a_new_page_when_full() {
        let mut a = allocator();
        // Fill the first page exactly: PAGE / ALIGN = 64 allocations of 16 bytes.
        let n = (PAGE / ALIGN) as usize;
        let mut allocs = Vec::new();
        for _ in 0..n {
            allocs.push(a.alloc(16).unwrap());
        }
        assert_eq!(a.page_count(), 1);
        assert_eq!(a.free_bytes(), 0, "first page should be exactly full");

        // The next allocation cannot fit, so a second page is added.
        let overflow = a.alloc(16).unwrap();
        assert_eq!(a.page_count(), 2);
        assert_eq!(overflow.page(), 1);
        assert_eq!(overflow.offset(), 0);
    }

    #[test]
    fn allocation_larger_than_a_page_fails() {
        let mut a = allocator();
        assert!(a.alloc(PAGE + 1).is_none());
        // A page-sized request (after alignment) fits exactly.
        assert!(a.alloc(PAGE).is_some());
        assert_eq!(a.free_bytes(), 0);
    }

    #[test]
    fn free_bytes_tracks_alloc_and_free() {
        let mut a = allocator();
        assert_eq!(a.free_bytes(), 0); // no pages yet
        let x = a.alloc(64).unwrap();
        assert_eq!(a.free_bytes(), PAGE - 64);
        a.free(x);
        assert_eq!(a.free_bytes(), PAGE);
        assert_eq!(a.live_count(), 0);
    }

    #[test]
    fn double_free_is_ignored() {
        let mut a = allocator();
        let x = a.alloc(32).unwrap();
        a.free(x);
        let before = a.free_bytes();
        a.free(x); // second free must be a no-op, not corrupt the free list
        assert_eq!(a.free_bytes(), before);
        assert_eq!(a.live_count(), 0);
    }

    #[test]
    fn foreign_handle_free_is_ignored() {
        let mut a = allocator();
        let _ = a.alloc(32).unwrap();
        // A handle the allocator never issued (same module, so we can fabricate one)
        // must not disturb bookkeeping.
        let fake = Allocation {
            page: 9,
            offset: 512,
            len: 16,
            span: 16,
        };
        a.free(fake);
        assert_eq!(a.live_count(), 1);
    }

    #[test]
    fn reset_frees_everything_but_keeps_pages() {
        let mut a = allocator();
        for _ in 0..3 {
            let _ = a.alloc(16).unwrap();
        }
        // Force a second page.
        for _ in 0..(PAGE / ALIGN) {
            let _ = a.alloc(16);
        }
        let pages_before = a.page_count();
        assert!(pages_before >= 2);
        a.reset();
        assert_eq!(a.live_count(), 0);
        assert_eq!(a.page_count(), pages_before, "reset keeps pages for reuse");
        assert_eq!(a.free_bytes(), PAGE * pages_before as u64);
    }
}
