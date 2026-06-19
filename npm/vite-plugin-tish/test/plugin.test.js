import path from "node:path";
import { describe, expect, it, vi } from "vitest";

import tishPlugin from "../index.js";
import { fixtureRoot, tishPath } from "./tish-path.js";

const mainTish = path.join(fixtureRoot, "src", "main.tish");
const counterTish = path.join(fixtureRoot, "src", "counter.tish");

function plugin(opts = {}) {
  return tishPlugin({ tishPath: tishPath(), projectRoot: fixtureRoot, ...opts });
}

describe("resolveId", () => {
  it("resolves a relative .tish import to an absolute path", () => {
    const id = plugin().resolveId("./counter.tish", mainTish);
    expect(id).toBe(counterTish);
  });

  it("passes through an absolute .tish path unchanged", () => {
    expect(plugin().resolveId(counterTish, undefined)).toBe(counterTish);
  });

  it("ignores non-.tish specifiers", () => {
    expect(plugin().resolveId("lattish", mainTish)).toBeNull();
    expect(plugin().resolveId("./styles.css", mainTish)).toBeNull();
  });
});

describe("load", () => {
  it("compiles a .tish module to ESM", () => {
    const out = plugin().load(counterTish);
    expect(out.code).toContain("export function makeCounter");
  });

  it("injects a self-accepting HMR boundary", () => {
    const out = plugin().load(counterTish);
    expect(out.code).toContain("import.meta.hot.accept()");
  });

  it("returns a v3 source map back to the .tish file", () => {
    const out = plugin().load(counterTish);
    expect(out.map).toBeTruthy();
    expect(out.map.version).toBe(3);
    expect(out.map.sources.some((s) => s.includes("counter.tish"))).toBe(true);
  });

  it("keeps relative .tish import specifiers so they stay in Vite's graph", () => {
    const out = plugin().load(mainTish);
    expect(out.code).toContain('from "./counter.tish"');
    expect(out.code).not.toContain("./counter.js");
  });

  it("omits the source map in full-reload mode", () => {
    const out = plugin({ mode: "full-reload" }).load(counterTish);
    expect(out.code).toContain("export function makeCounter");
    expect(out.code).not.toContain("import.meta.hot.accept()");
    expect(out.map).toBeNull();
  });

  it("ignores non-.tish ids", () => {
    expect(plugin().load("/some/file.js")).toBeNull();
  });
});

describe("handleHotUpdate", () => {
  it("returns the changed module node(s) instead of a full reload (HMR)", () => {
    const send = vi.fn();
    const node = { id: counterTish };
    const server = {
      ws: { send },
      moduleGraph: { getModulesByFile: () => new Set([node]) },
    };
    const result = plugin().handleHotUpdate({ file: counterTish, server });
    expect(result).toEqual([node]);
    expect(send).not.toHaveBeenCalledWith({ type: "full-reload" });
  });

  it("sends a full reload only in full-reload mode", () => {
    const send = vi.fn();
    const server = { ws: { send }, moduleGraph: { getModulesByFile: () => undefined } };
    const result = plugin({ mode: "full-reload" }).handleHotUpdate({
      file: counterTish,
      server,
    });
    expect(result).toEqual([]);
    expect(send).toHaveBeenCalledWith({ type: "full-reload" });
  });

  it("ignores non-.tish files", () => {
    const send = vi.fn();
    const server = { ws: { send }, moduleGraph: { getModulesByFile: () => undefined } };
    expect(plugin().handleHotUpdate({ file: "/x/app.css", server })).toBeUndefined();
  });
});
