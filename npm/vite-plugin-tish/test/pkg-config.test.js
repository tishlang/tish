import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import tishPlugin, { readPkgTishConfig } from "../index.js";
import { tishPath } from "./tish-path.js";

const tempDirs = [];

afterEach(() => {
  while (tempDirs.length) {
    fs.rmSync(tempDirs.pop(), { recursive: true, force: true });
  }
});

describe("readPkgTishConfig", () => {
  it("reads tish.platform / tish.surface", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "tish-vite-pkg-"));
    tempDirs.push(root);
    fs.writeFileSync(
      path.join(root, "package.json"),
      JSON.stringify({ name: "x", tish: { platform: "web", surface: "web" } }),
    );
    expect(readPkgTishConfig(root)).toEqual({ platform: "web", surface: "web" });
  });

  it("reads nested tish.desktop.*", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "tish-vite-pkg-"));
    tempDirs.push(root);
    fs.writeFileSync(
      path.join(root, "package.json"),
      JSON.stringify({
        name: "x",
        tish: { desktop: { platform: "macos", surface: "webview" } },
      }),
    );
    expect(readPkgTishConfig(root)).toEqual({
      platform: "macos",
      surface: "webview",
    });
  });
});

describe("package.json platform/surface via configResolved", () => {
  it("uses package.json when opts and env are unset", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "tish-vite-pkg-"));
    tempDirs.push(root);
    fs.writeFileSync(path.join(root, "Button.tish"), "export fn Button() {}\n");
    fs.writeFileSync(path.join(root, "Button.web.tish"), "export fn Button() {}\n");
    fs.writeFileSync(path.join(root, "Button.macos.tish"), "export fn Button() {}\n");
    const importer = path.join(root, "App.tish");
    fs.writeFileSync(importer, 'import { Button } from "./Button"\n');
    fs.writeFileSync(
      path.join(root, "package.json"),
      JSON.stringify({ name: "x", tish: { platform: "web", surface: "web" } }),
    );

    const prevP = process.env.TISH_PLATFORM;
    const prevS = process.env.TISH_SURFACE;
    delete process.env.TISH_PLATFORM;
    delete process.env.TISH_SURFACE;
    try {
      const plugin = tishPlugin({
        tishPath: tishPath(),
        projectRoot: root,
      });
      plugin.configResolved({ root });
      const id = plugin.resolveId("./Button", importer);
      expect(id).toBeTruthy();
      expect(path.basename(id)).toBe("Button.web.tish");
    } finally {
      if (prevP !== undefined) process.env.TISH_PLATFORM = prevP;
      else delete process.env.TISH_PLATFORM;
      if (prevS !== undefined) process.env.TISH_SURFACE = prevS;
      else delete process.env.TISH_SURFACE;
    }
  });
});
