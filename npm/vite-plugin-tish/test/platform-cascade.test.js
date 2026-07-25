/**
 * Plan 0a exit: Vite resolveId and `tish resolve-id` share the same platform/surface cascade.
 * Same fixture shape as `tishlang_compile` platform_resolve_cli goldens.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import tishPlugin from "../index.js";
import { tishPath } from "./tish-path.js";

const tempDirs = [];

afterEach(() => {
  while (tempDirs.length) {
    fs.rmSync(tempDirs.pop(), { recursive: true, force: true });
  }
});

function makeCascadeFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tish-vite-cascade-"));
  tempDirs.push(root);
  fs.writeFileSync(path.join(root, "Button.tish"), "export fn Button() {}\n");
  fs.writeFileSync(path.join(root, "Button.web.tish"), "export fn Button() {}\n");
  fs.writeFileSync(path.join(root, "Button.webview.tish"), "export fn Button() {}\n");
  fs.writeFileSync(path.join(root, "Button.macos.tish"), "export fn Button() {}\n");
  fs.writeFileSync(path.join(root, "Button.desktop.tish"), "export fn Button() {}\n");
  const importer = path.join(root, "App.tish");
  fs.writeFileSync(importer, 'import { Button } from "./Button"\n');
  return { root, importer };
}

function plugin(platform, surface) {
  const bin = tishPath();
  if (!fs.existsSync(bin) && bin !== "tish") {
    throw new Error(
      `tish binary required for platform cascade test (TISH_PATH=${bin}). Soft-skip forbidden.`,
    );
  }
  return tishPlugin({
    tishPath: bin,
    platform,
    surface,
  });
}

describe("platform/surface cascade via resolveId", () => {
  it("macos + native → Button.macos.tish", () => {
    const { importer } = makeCascadeFixture();
    const id = plugin("macos", "native").resolveId("./Button", importer);
    expect(id).toBeTruthy();
    expect(path.basename(id)).toBe("Button.macos.tish");
  });

  it("macos + webview → Button.webview.tish (before .web)", () => {
    const { importer } = makeCascadeFixture();
    const id = plugin("macos", "webview").resolveId("./Button", importer);
    expect(id).toBeTruthy();
    expect(path.basename(id)).toBe("Button.webview.tish");
  });

  it("web + web → Button.web.tish", () => {
    const { importer } = makeCascadeFixture();
    const id = plugin("web", "web").resolveId("./Button", importer);
    expect(id).toBeTruthy();
    expect(path.basename(id)).toBe("Button.web.tish");
  });

  it("remaps explicit ./Button.tish to platform file when present", () => {
    const { importer } = makeCascadeFixture();
    const id = plugin("macos", "native").resolveId("./Button.tish", importer);
    expect(id).toBeTruthy();
    expect(path.basename(id)).toBe("Button.macos.tish");
  });
});
