import { describe, expect, it } from "vitest";
import { ancestorsWithin } from "./explorer-ui";

describe("ancestorsWithin", () => {
  it("returns ancestor dirs strictly below the root, shallow-first", () => {
    expect(ancestorsWithin("/root/a/b/file.md", "/root")).toEqual(["/root/a", "/root/a/b"]);
  });

  it("returns [] for a file directly in the root", () => {
    expect(ancestorsWithin("/root/file.md", "/root")).toEqual([]);
  });

  it("returns [] for paths outside the root", () => {
    expect(ancestorsWithin("/elsewhere/file.md", "/root")).toEqual([]);
  });

  it("tolerates a trailing slash on the root", () => {
    expect(ancestorsWithin("/root/a/file.md", "/root/")).toEqual(["/root/a"]);
  });
});
