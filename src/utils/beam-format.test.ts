import { describe, expect, it } from "vitest";
import { expiresIn, humanBytes } from "./beam-format";

describe("humanBytes", () => {
  it("scales through the units", () => {
    expect(humanBytes(0)).toBe("0 B");
    expect(humanBytes(512)).toBe("512 B");
    expect(humanBytes(482_133)).toBe("471 KB");
    expect(humanBytes(1_300_000)).toBe("1.2 MB");
    expect(humanBytes(250 * 1024 * 1024)).toBe("250 MB");
  });

  it("rejects garbage", () => {
    expect(humanBytes(-1)).toBe("?");
    expect(humanBytes(Number.NaN)).toBe("?");
  });
});

describe("expiresIn", () => {
  const now = 1_000_000;
  it("coarsens by magnitude", () => {
    expect(expiresIn(now + 23 * 3600, now)).toBe("23 h");
    expect(expiresIn(now + 45 * 60, now)).toBe("45 min");
    expect(expiresIn(now + 30, now)).toBe("<1 min");
    expect(expiresIn(now, now)).toBe("expired");
    expect(expiresIn(now - 5, now)).toBe("expired");
  });
});
