// Ground-truth probe: what can headless Chromium on THIS host actually do?
// Reports, for a no-flags launch and a WebGPU-flagged launch, whether
// navigator.gpu exists, whether requestAdapter() returns a real adapter, and
// whether WebGL2 is available (with its renderer string). This decides what an
// honest e2e suite can assert. It does not touch the Reticle app.
import { chromium } from "@playwright/test";

async function probe(label, args, headless = true) {
  const browser = await chromium.launch({ headless, args });
  const page = await browser.newPage();
  await page.setContent("<!doctype html><title>probe</title><canvas id=c></canvas>");
  const result = await page.evaluate(async () => {
    const out = { gpuInNavigator: "gpu" in navigator, adapter: null, adapterInfo: null, webgl2: false, webgl2Renderer: null };
    if ("gpu" in navigator) {
      try {
        const a = await navigator.gpu.requestAdapter();
        out.adapter = a ? true : false;
        if (a) {
          try {
            const info = a.info || (a.requestAdapterInfo ? await a.requestAdapterInfo() : null);
            out.adapterInfo = info ? { vendor: info.vendor, architecture: info.architecture, device: info.device } : "adapter-present";
          } catch (e) { out.adapterInfo = "info-unavailable"; }
        }
      } catch (e) { out.adapter = "requestAdapter-threw: " + String(e); }
    }
    try {
      const gl = document.getElementById("c").getContext("webgl2");
      out.webgl2 = !!gl;
      if (gl) {
        const dbg = gl.getExtension("WEBGL_debug_renderer_info");
        out.webgl2Renderer = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : "renderer-hidden";
      }
    } catch (e) { out.webgl2 = "getContext-threw: " + String(e); }
    return out;
  });
  console.log(`\n[${label}] args=${JSON.stringify(args)}`);
  console.log(JSON.stringify(result, null, 2));
  await browser.close();
  return result;
}

const WEBGPU_ARGS = [
  "--enable-unsafe-webgpu",
  "--enable-features=Vulkan,WebGPU",
  "--use-angle=swiftshader",
  "--use-gl=angle",
];

await probe("no-flags", []);
await probe("webgpu-flagged", WEBGPU_ARGS);
// New headless (--headless=new) has a real GPU process; best chance at a
// SwiftShader Dawn WebGPU adapter. Launched via headless:false so our own
// --headless=new arg controls the mode.
await probe("new-headless-webgpu", ["--headless=new", "--no-sandbox", ...WEBGPU_ARGS], false);
console.log("\nprobe done");
