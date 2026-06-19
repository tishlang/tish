import path from "node:path";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { createServer } from "vite";

import tishPlugin from "../index.js";
import { fixtureRoot, tishPath } from "./tish-path.js";

// Acceptance test for #284: a real Vite dev server resolves, loads, and transforms `.tish` modules
// through the plugin (proving in-graph compilation), and a leaf edit yields per-module HMR rather
// than a full page reload.
describe("vite dev server", () => {
  const counterTish = path.join(fixtureRoot, "src", "counter.tish");
  const myPlugin = tishPlugin({ tishPath: tishPath(), projectRoot: fixtureRoot });
  let server;

  beforeAll(async () => {
    server = await createServer({
      root: fixtureRoot,
      logLevel: "silent",
      server: { middlewareMode: true, hmr: false },
      optimizeDeps: { noDiscovery: true },
      plugins: [myPlugin],
    });
  });

  afterAll(async () => {
    await server?.close();
  });

  it("transforms a .tish entry into ESM and pulls its .tish import into the graph", async () => {
    const result = await server.transformRequest("/src/main.tish");
    expect(result).toBeTruthy();
    expect(result.code).toContain("makeCounter");
    // Import analysis resolved `./counter.tish` through the plugin and registered it as a module.
    const counterMods = server.moduleGraph.getModulesByFile(counterTish);
    expect(counterMods && counterMods.size).toBeGreaterThan(0);
  });

  it("HMR returns the changed module and sends no full-reload", async () => {
    await server.transformRequest("/src/counter.tish");
    const send = vi.spyOn(server.ws, "send");
    const updated = myPlugin.handleHotUpdate({ file: counterTish, server });
    expect(Array.isArray(updated)).toBe(true);
    expect(updated.length).toBeGreaterThan(0);
    const sentFullReload = send.mock.calls.some(
      (args) => args[0] && args[0].type === "full-reload",
    );
    expect(sentFullReload).toBe(false);
  });
});
