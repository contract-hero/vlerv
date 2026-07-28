import { describe, expect, it } from "vitest";
import { openOptsFromClick } from "./open-opts";

describe("openOptsFromClick", () => {
  it("plain click (and plain shift-click) navigate in the same tab", () => {
    expect(openOptsFromClick({})).toBeUndefined();
    expect(openOptsFromClick({ shiftKey: true })).toBeUndefined();
    expect(openOptsFromClick({ button: 0 })).toBeUndefined();
  });

  it("meta/ctrl-click opens a background tab; +shift foregrounds it", () => {
    expect(openOptsFromClick({ metaKey: true })).toEqual({ newTab: true, background: true });
    expect(openOptsFromClick({ ctrlKey: true })).toEqual({ newTab: true, background: true });
    expect(openOptsFromClick({ metaKey: true, shiftKey: true })).toEqual({
      newTab: true,
      background: false,
    });
  });

  it("middle-click opens a background tab", () => {
    expect(openOptsFromClick({ button: 1 })).toEqual({ newTab: true, background: true });
  });
});
