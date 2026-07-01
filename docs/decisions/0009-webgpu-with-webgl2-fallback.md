# 0009 — WebGPU primary with WebGL2 fallback

## Context

The browser demo must reach a wide audience while showcasing modern GPU-driven
rendering. WebGPU is the target API (compute shaders enable GPU-driven culling) but
is not yet universal across browsers and drivers. `wgpu` can target both WebGPU and
WebGL2 from one codebase.

## Decision

Default to WebGPU in the browser and natively (Vulkan/Metal/DX12). Provide a WebGL2
fallback path in `wgpu` for reach, with a runtime capability check in the `web`
harness that selects WebGPU when available and otherwise falls back with a clear
message. Features that require compute (GPU-driven culling) degrade to a CPU-built
draw list on the WebGL2 path so the demo still runs.

## Consequences

The demo runs for the broadest set of visitors, and the flagship path
(WebGPU + compute culling) is unblocked. The cost is maintaining two rendering
paths for the culling stage and testing both. The capability check and the
degraded path are documented in the rendering chapter.
