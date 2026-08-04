import { describe, expect, it } from "vitest";
import { readErrorTitle } from "./read-error";

// The strings below are the real `thiserror` Display output from
// src-tauri/src/reader.rs and src-tauri/src/security.rs, not invented text.
describe("readErrorTitle", () => {
  it("names the common failures", () => {
    expect(readErrorTitle("Io", 'not found: "/w/report.html"')).toBe("File not found");
    expect(
      readErrorTitle("Io", 'io error at "/w/report.html": Permission denied (os error 13)'),
    ).toBe("Permission denied");
    expect(readErrorTitle("Io", 'path is out of root: "/etc/passwd"')).toBe(
      "Outside the workspace",
    );
  });

  it("classifies on the failure, never on the path text", () => {
    // A quoted path can contain any word we match on. `NotFound.tsx` is a
    // very common filename; reporting it as missing while it sits on disk
    // would be worse than the generic fallback.
    expect(
      readErrorTitle(
        "Io",
        'io error at "/repo/src/pages/NotFound.tsx": Permission denied (os error 13)',
      ),
    ).toBe("Permission denied");
    expect(readErrorTitle("Io", 'not found: "/repo/permission-denied/a.md"')).toBe(
      "File not found",
    );
  });

  it("prefers the more specific classification when both could match", () => {
    // "out of root" must win over the "denied" substring.
    expect(readErrorTitle("Io", 'path is out of root: "/denied/x.md"')).toBe(
      "Outside the workspace",
    );
  });

  it("falls back rather than guessing", () => {
    expect(readErrorTitle("Io", "")).toBe("Could not read this file");
    expect(readErrorTitle("Io", "some unmapped backend failure")).toBe(
      "Could not read this file",
    );
  });
});
