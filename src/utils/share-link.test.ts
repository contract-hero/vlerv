import { describe, expect, it, vi } from "vitest";
import {
  isShareAbort,
  shareLinkRoute,
  shareLinkViaWebShare,
  type WebShareTarget,
} from "./share-link";

const LINK = "vlerv://pair?ticket=ABC";

function abortError(): Error {
  const err = new Error("dismissed");
  err.name = "AbortError";
  return err;
}

describe("shareLinkRoute", () => {
  it("uses the native command on macOS", () => {
    expect(shareLinkRoute(false, { shareLink: async () => {} }, {})).toBe("native");
  });

  it("reports unavailable on macOS when the command is not wired", () => {
    expect(shareLinkRoute(false, {}, { share: async () => {} })).toBe("unavailable");
  });

  it("uses Web Share on iOS", () => {
    expect(shareLinkRoute(true, {}, { share: async () => {} })).toBe("web-share");
  });

  it("reports unavailable on iOS when the webview has no Web Share", () => {
    // The command exists on every platform but answers with an error there,
    // so it must not be offered as a fallback.
    expect(shareLinkRoute(true, { shareLink: async () => {} }, {})).toBe("unavailable");
    expect(shareLinkRoute(true, {}, undefined)).toBe("unavailable");
  });
});

describe("isShareAbort", () => {
  it("recognizes the user dismissing the sheet", () => {
    expect(isShareAbort(abortError())).toBe(true);
  });

  it("does not swallow other failures", () => {
    expect(isShareAbort(new TypeError("bad url"))).toBe(false);
    expect(isShareAbort(null)).toBe(false);
    expect(isShareAbort("AbortError")).toBe(false);
  });
});

describe("shareLinkViaWebShare", () => {
  it("shares the link as a URL, which is what offers AirDrop", async () => {
    const share = vi.fn().mockResolvedValue(undefined);
    const canShare = vi.fn().mockReturnValue(true);
    await shareLinkViaWebShare({ canShare, share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledTimes(1);
    expect(share).toHaveBeenCalledWith({ title: "Pair", url: LINK });
  });

  it("falls back to text when the webview will not take the scheme as a URL", async () => {
    const share = vi.fn().mockResolvedValue(undefined);
    const canShare = vi.fn().mockReturnValue(false);
    await shareLinkViaWebShare({ canShare, share } as WebShareTarget, LINK, "Pair");
    expect(canShare).toHaveBeenCalledWith({ title: "Pair", url: LINK });
    expect(share).toHaveBeenCalledTimes(1);
    expect(share).toHaveBeenCalledWith({ title: "Pair", text: LINK });
  });

  it("never calls share twice — the second call would have no user activation", async () => {
    // The regression this guards: share() consumes transient activation, so a
    // retry after a rejection always fails with NotAllowedError. One click,
    // one call, whatever the outcome.
    const share = vi.fn().mockRejectedValue(new TypeError("unsupported URL"));
    const canShare = vi.fn().mockReturnValue(true);
    await expect(
      shareLinkViaWebShare({ canShare, share } as WebShareTarget, LINK, "Pair"),
    ).rejects.toThrow(TypeError);
    expect(share).toHaveBeenCalledTimes(1);
  });

  it("prefers the URL when the webview cannot be asked", async () => {
    // No canShare: url is the payload the feature exists for, so it is the
    // better guess.
    const share = vi.fn().mockResolvedValue(undefined);
    await shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledWith({ title: "Pair", url: LINK });
  });

  it("treats a dismissal as success, so the button reports nothing", async () => {
    const share = vi.fn().mockRejectedValue(abortError());
    await shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledTimes(1);
  });

  it("propagates every other failure so the caller can surface it", async () => {
    const share = vi.fn().mockRejectedValue(new DOMException("no activation", "NotAllowedError"));
    await expect(
      shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair"),
    ).rejects.toThrow(/no activation/);
  });

  it("rejects when the webview has no Web Share at all", async () => {
    await expect(shareLinkViaWebShare(undefined, LINK, "Pair")).rejects.toThrow(/Web Share/);
  });
});
