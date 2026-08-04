import { describe, expect, it } from "vitest";
import { basename, dirname, displayDir, isUnderRoot, normalizePathBarInput } from "./path";

describe("basename / dirname", () => {
  it("splits regular paths", () => {
    expect(basename("/a/b/c.html")).toBe("c.html");
    expect(dirname("/a/b/c.html")).toBe("/a/b");
  });
  it("handles root-level paths", () => {
    expect(basename("/file")).toBe("file");
    expect(dirname("/file")).toBe("/");
  });
});

describe("isUnderRoot", () => {
  it("matches the root itself and children, not prefix-sibling dirs", () => {
    expect(isUnderRoot("/foo/bar", "/foo/bar")).toBe(true);
    expect(isUnderRoot("/foo/bar/x.md", "/foo/bar")).toBe(true);
    expect(isUnderRoot("/foo/barn/x.md", "/foo/bar")).toBe(false);
    expect(isUnderRoot("/anything", null)).toBe(false);
  });
});

describe("normalizePathBarInput", () => {
  it("passes raw absolute paths through literally (no decoding)", () => {
    expect(normalizePathBarInput("/tmp/a.html")).toEqual({ kind: "ok", path: "/tmp/a.html" });
    // Real files can contain % — pasting from Finder/pwd must not decode.
    expect(normalizePathBarInput("/tmp/100%.md")).toEqual({ kind: "ok", path: "/tmp/100%.md" });
    expect(normalizePathBarInput("/tmp/a%20b.md")).toEqual({ kind: "ok", path: "/tmp/a%20b.md" });
  });

  it("decodes file:// URLs exactly once", () => {
    expect(normalizePathBarInput("file:///tmp/a%20b.html")).toEqual({
      kind: "ok",
      path: "/tmp/a b.html",
    });
  });

  it("decodes the vlerv:// path param exactly once", () => {
    expect(normalizePathBarInput("vlerv://open?path=%2Ftmp%2Fa%20b.md")).toEqual({
      kind: "ok",
      path: "/tmp/a b.md",
    });
    // Double-encoded input stays single-encoded after one decode — matching
    // dispatch_deep_link, a file literally named "a%20b.md" is addressable.
    expect(normalizePathBarInput("vlerv://open?path=%2Ftmp%2Fa%2520b.md")).toEqual({
      kind: "ok",
      path: "/tmp/a%20b.md",
    });
  });

  it("does not form-decode + to a space (Rust parser parity)", () => {
    expect(normalizePathBarInput("vlerv://open?path=/tmp/a+b.md")).toEqual({
      kind: "ok",
      path: "/tmp/a+b.md",
    });
  });

  it("ignores other params and accepts reveal", () => {
    expect(normalizePathBarInput("vlerv://open?path=/tmp/x.md&line=42")).toEqual({
      kind: "ok",
      path: "/tmp/x.md",
    });
    expect(normalizePathBarInput("vlerv://reveal?path=/tmp/x.md")).toEqual({
      kind: "ok",
      path: "/tmp/x.md",
    });
  });

  it("rejects malformed inputs", () => {
    expect(normalizePathBarInput("").kind).toBe("error");
    expect(normalizePathBarInput("   ").kind).toBe("error");
    expect(normalizePathBarInput("relative/path.md").kind).toBe("error");
    expect(normalizePathBarInput("vlerv://open").kind).toBe("error");
    expect(normalizePathBarInput("vlerv://open?foo=1").kind).toBe("error");
    expect(normalizePathBarInput("vlerv://open?path=%ZZ").kind).toBe("error");
    expect(normalizePathBarInput("vlerv://open?path=/tmp/a%00b").kind).toBe("error");
  });
});

describe("displayDir", () => {
  it("shows a path relative to the workspace root", () => {
    expect(displayDir("/w/vlerv/src/components/a.tsx", "/w/vlerv")).toBe("src/components");
  });

  it("shows nothing for a file sitting at the root", () => {
    expect(displayDir("/w/vlerv/README.md", "/w/vlerv")).toBe("");
  });

  it("never fakes a relative path for a prefix sibling", () => {
    // `/w/vlerv-worktrees/x` is NOT inside `/w/vlerv`. Without the trailing
    // slash in the prefix test this returns "worktrees/x", a path that does
    // not exist. Git worktrees produce exactly this layout.
    expect(displayDir("/w/vlerv-worktrees/x/a.md", "/w/vlerv")).toBe("/w/vlerv-worktrees/x");
  });

  it("falls back to the absolute directory outside the root or with no root", () => {
    expect(displayDir("/tmp/a.md", "/w/vlerv")).toBe("/tmp");
    expect(displayDir("/w/vlerv/src/a.md", null)).toBe("/w/vlerv/src");
  });
});
