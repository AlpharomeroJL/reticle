//! GPU device acquisition.
//!
//! [`WgpuContext`] owns a headless wgpu instance, adapter, device, and queue with no
//! surface. It is the entry point every render path builds on. Construction is
//! async (adapter and device requests are futures); [`WgpuContext::new_blocking`]
//! wraps it with `pollster` on native targets.

use wgpu::{
    Adapter, Device, DeviceDescriptor, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, Queue, RequestAdapterOptions,
};

/// A headless GPU context: instance, adapter, device, and queue with no surface.
///
/// Clone the `device`/`queue` (both are cheap handles) into per-target state, or
/// pass the context by reference to the render entry points.
pub struct WgpuContext {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
}

impl core::fmt::Debug for WgpuContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WgpuContext")
            .field("backend", &self.adapter.get_info().backend)
            .field("adapter", &self.adapter.get_info().name)
            .finish_non_exhaustive()
    }
}

impl WgpuContext {
    /// Acquires a headless context, or `None` if no adapter is available.
    ///
    /// Enables all default backends and requests a high-performance adapter with no
    /// surface. Returns `None` when the platform exposes no compatible GPU (for
    /// example CI without a software rasterizer), so callers can skip GPU work
    /// gracefully.
    pub async fn new() -> Option<Self> {
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .ok()?;

        let (device, queue) = adapter
            .request_device(&DeviceDescriptor {
                label: Some("reticle-render device"),
                required_features: wgpu::Features::empty(),
                // Keep to the conservative default limit set so headless native and
                // the browser's WebGPU both satisfy the request.
                required_limits: Limits::default(),
                memory_hints: MemoryHints::Performance,
                ..Default::default()
            })
            .await
            .ok()?;

        Some(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Acquires a headless context, blocking the current thread until the async
    /// initialization completes. Returns `None` if no adapter is available.
    ///
    /// Native only: this uses `pollster::block_on`, which cannot run on
    /// `wasm32-unknown-unknown` (there is no thread to block). On the web, await
    /// [`WgpuContext::new`] from an async context instead.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn new_blocking() -> Option<Self> {
        pollster::block_on(Self::new())
    }

    /// The wgpu instance.
    #[must_use]
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// The selected adapter.
    #[must_use]
    pub fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    /// The logical device.
    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The command queue.
    #[must_use]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
}
