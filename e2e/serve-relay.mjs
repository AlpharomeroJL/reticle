// Launches the reticle-server collaboration relay for the share-live e2e, and
// exposes a tiny HTTP health endpoint Playwright's webServer waits on.
//
// The relay itself only speaks `GET /ws/{room}` (a WebSocket upgrade), so it is not a
// URL Playwright can poll for readiness. This wrapper spawns the prebuilt relay binary
// as a child on the relay port (default 3030) and runs a trivial HTTP server on a
// separate health port (default 3031) that returns 200, so Playwright can confirm the
// relay came up before the specs run. Killing this process stops the relay child too.
//
// The relay binary must already be built (see `just e2e-share`); this does not compile.
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";

const here = fileURLToPath(new URL(".", import.meta.url));
const RELAY_PORT = Number(process.env.RELAY_PORT || 3030);
const HEALTH_PORT = Number(process.env.RELAY_HEALTH_PORT || 3031);
const HOST = process.env.HOST || "127.0.0.1";

// The relay binary lives in the workspace target dir. Honor CARGO_TARGET_DIR (the lane
// build uses a lane-specific target), else fall back to the in-repo target/.
const targetDir =
  process.env.CARGO_TARGET_DIR || join(here, "..", "target");
const exe = process.platform === "win32" ? "reticle-server.exe" : "reticle-server";
const candidates = [
  join(targetDir, "release", exe),
  join(targetDir, "debug", exe),
];
const bin = candidates.find((p) => existsSync(p));
if (!bin) {
  console.error(
    `serve-relay: could not find the reticle-server binary in:\n  ${candidates.join(
      "\n  ",
    )}\nBuild it first (e.g. \`cargo build -p reticle-server\`).`,
  );
  process.exit(1);
}

console.log(`serve-relay: launching ${bin} on ${HOST}:${RELAY_PORT}`);
const relay = spawn(bin, [], {
  env: { ...process.env, RETICLE_SERVER_ADDR: `${HOST}:${RELAY_PORT}` },
  stdio: "inherit",
});
relay.on("exit", (code) => {
  console.error(`serve-relay: relay exited with code ${code}`);
  process.exit(code ?? 1);
});

// Health endpoint Playwright waits on.
const health = createServer((_req, res) => {
  res.writeHead(200, { "content-type": "text/plain" });
  res.end(`relay up on ${HOST}:${RELAY_PORT}`);
});
health.listen(HEALTH_PORT, HOST, () => {
  console.log(`serve-relay: health on http://${HOST}:${HEALTH_PORT}`);
});

// Clean shutdown: stop the relay child when this wrapper is terminated.
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    relay.kill();
    process.exit(0);
  });
}
