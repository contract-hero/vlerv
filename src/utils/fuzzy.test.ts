import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("returns null when the query is not a subsequence", () => {
    expect(fuzzyScore("zzz", "src/app.tsx")).toBeNull();
  });

  it("empty query matches everything with score 0", () => {
    expect(fuzzyScore("", "anything")).toBe(0);
  });

  it("prefers basename matches over deep-path matches", () => {
    const results = fuzzyFilter("app", ["src/app.tsx", "apples/other/readme.md"], 10);
    expect(results[0].value).toBe("src/app.tsx");
  });

  it("prefers boundary-aligned matches", () => {
    const results = fuzzyFilter("tb", ["components/TabStrip.tsx", "scratch/notebook.txt"], 10);
    expect(results[0].value).toBe("components/TabStrip.tsx");
  });

  it("is case-insensitive", () => {
    expect(fuzzyScore("README", "docs/readme.md")).not.toBeNull();
  });
});
