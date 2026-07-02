//! GPU-backed smoke test for [`BufferPages`].
//!
//! Confirms the page bank creates real `wgpu` buffers, grows one page at a time as
//! allocations overflow a page, and reuses freed ranges without creating another
//! buffer. Like the golden test, it skips (and passes) when no GPU adapter is
//! available, so it is safe in CI.

use reticle_render::{BufferPages, WgpuContext};
use wgpu::BufferUsages;

/// A small page so a couple of allocations force growth without huge uploads.
const PAGE: u64 = 4096;

#[test]
fn buffer_pages_grow_and_reuse_on_gpu() {
    let Some(ctx) = WgpuContext::new_blocking() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let device = ctx.device();
    let queue = ctx.queue();

    let mut pages = BufferPages::new(PAGE, BufferUsages::VERTEX, "test pages");
    assert_eq!(pages.page_count(), 0, "no buffers before the first upload");

    // Two half-page uploads land in one page (creating exactly one buffer).
    let half = vec![0xABu8; (PAGE / 2) as usize];
    let a = pages.upload(device, queue, &half).expect("alloc a");
    let b = pages.upload(device, queue, &half).expect("alloc b");
    assert_eq!(pages.page_count(), 1, "two halves share one page buffer");
    assert_eq!(a.page(), 0);
    assert_eq!(b.page(), 0);
    assert!(pages.page_buffer(0).is_some());

    // A third half-page upload does not fit, so the bank grows by one buffer.
    let c = pages.upload(device, queue, &half).expect("alloc c");
    assert_eq!(
        pages.page_count(),
        2,
        "overflow adds exactly one page buffer"
    );
    assert_eq!(c.page(), 1);

    // Free b and re-upload the same size: it reuses b's range in page 0, no growth.
    pages.free(b);
    let d = pages.upload(device, queue, &half).expect("alloc d");
    assert_eq!(d.page(), 0, "reuse fills the freed range in the first page");
    assert_eq!(
        pages.page_count(),
        2,
        "reuse must not create another buffer"
    );

    // Drive the queue so the writes are observed (and the test exercises submission).
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}
