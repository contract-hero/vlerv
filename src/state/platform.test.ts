import { describe, expect, it } from "vitest";
import { parsePlatformOs, platformBodyClass, resolvePlatformOs } from "./platform";

describe("parsePlatformOs", () => {
  it("recognizes ios", () => {
    expect(parsePlatformOs({ os: "ios" })).toBe("ios");
  });

  it("recognizes macos", () => {
    expect(parsePlatformOs({ os: "macos" })).toBe("macos");
  });

  it("falls back to macos for an unknown os value", () => {
    expect(parsePlatformOs({ os: "windows" })).toBe("macos");
  });

  it("falls back to macos for null, undefined, or non-object input", () => {
    expect(parsePlatformOs(null)).toBe("macos");
    expect(parsePlatformOs(undefined)).toBe("macos");
    expect(parsePlatformOs("ios")).toBe("macos");
    expect(parsePlatformOs(42)).toBe("macos");
  });

  it("falls back to macos when the os field is missing", () => {
    expect(parsePlatformOs({})).toBe("macos");
  });
});

describe("resolvePlatformOs", () => {
  it("returns macos when platformInfo is not wired", async () => {
    await expect(resolvePlatformOs({})).resolves.toBe("macos");
  });

  it("returns macos when platformInfo rejects", async () => {
    const ipc = { platformInfo: () => Promise.reject(new Error("no such command")) };
    await expect(resolvePlatformOs(ipc)).resolves.toBe("macos");
  });

  it("returns ios when platformInfo resolves to ios", async () => {
    const ipc = { platformInfo: () => Promise.resolve({ os: "ios" as const }) };
    await expect(resolvePlatformOs(ipc)).resolves.toBe("ios");
  });

  it("returns macos when platformInfo resolves to an unexpected shape", async () => {
    const ipc = { platformInfo: () => Promise.resolve({ os: "android" } as never) };
    await expect(resolvePlatformOs(ipc)).resolves.toBe("macos");
  });

  it("returns macos when platformInfo throws synchronously", async () => {
    const ipc = {
      platformInfo: () => {
        throw new Error("boom");
      },
    };
    await expect(resolvePlatformOs(ipc)).resolves.toBe("macos");
  });
});

describe("platformBodyClass", () => {
  it("maps ios/macos to their body class", () => {
    expect(platformBodyClass("ios")).toBe("platform-ios");
    expect(platformBodyClass("macos")).toBe("platform-macos");
  });
});
