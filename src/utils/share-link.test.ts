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
    await shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledTimes(1);
    expect(share).toHaveBeenCalledWith({ title: "Pair", url: LINK });
  });

  it("retries as text when the webview rejects the scheme", async () => {
    const share = vi
      .fn()
      .mockRejectedValueOnce(new TypeError("unsupported URL"))
      .mockResolvedValueOnce(undefined);
    await shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledTimes(2);
    expect(share).toHaveBeenLastCalledWith({ title: "Pair", text: LINK });
  });

  it("does not retry after the user dismisses the sheet", async () => {
    const share = vi.fn().mockRejectedValue(abortError());
    await shareLinkViaWebShare({ share } as WebShareTarget, LINK, "Pair");
    expect(share).toHaveBeenCalledTimes(1);
  });

  it("rejects when the webview has no Web Share at all", async () => {
    await expect(shareLinkViaWebShare(undefined, LINK, "Pair")).rejects.toThrow(/Web Share/);
  });
});
