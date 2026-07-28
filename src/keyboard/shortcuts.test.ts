import { describe, expect, it } from "vitest";
import { dispatchChord, matchesCombo, parseCombo } from "./shortcuts";
import type { Binding, ChordEvent } from "./shortcuts";

function ev(partial: Partial<ChordEvent>): ChordEvent {
  return {
    code: "",
    key: "",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...partial,
  };
}

describe("parseCombo + matchesCombo (mac)", () => {
  const cases: Array<[string, ChordEvent]> = [
    ["mod+t", ev({ code: "KeyT", metaKey: true })],
    ["mod+bracketleft", ev({ code: "BracketLeft", metaKey: true })],
    ["mod+bracketright", ev({ code: "BracketRight", metaKey: true })],
    ["mod+shift+bracketleft", ev({ code: "BracketLeft", metaKey: true, shiftKey: true })],
    ["mod+shift+bracketright", ev({ code: "BracketRight", metaKey: true, shiftKey: true })],
    ["mod+equal", ev({ code: "Equal", metaKey: true })],
    ["mod+shift+equal", ev({ code: "Equal", metaKey: true, shiftKey: true })],
    ["mod+minus", ev({ code: "Minus", metaKey: true })],
    ["mod+digit1", ev({ code: "Digit1", metaKey: true })],
    ["mod+digit0", ev({ code: "Digit0", metaKey: true })],
    ["ctrl+tab", ev({ code: "Tab", ctrlKey: true })],
    ["ctrl+shift+tab", ev({ code: "Tab", ctrlKey: true, shiftKey: true })],
    ["mod+[", ev({ code: "BracketLeft", metaKey: true })],
    ["mod+KeyK", ev({ code: "KeyK", metaKey: true })],
  ];
  for (const [combo, event] of cases) {
    it(`${combo} fires`, () => {
      expect(matchesCombo(event, parseCombo(combo), true)).toBe(true);
    });
  }

  it("exact modifiers: mod+[ does not swallow mod+shift+[ and vice versa", () => {
    const plain = parseCombo("mod+bracketleft");
    const shifted = parseCombo("mod+shift+bracketleft");
    const shiftEvent = ev({ code: "BracketLeft", metaKey: true, shiftKey: true });
    const plainEvent = ev({ code: "BracketLeft", metaKey: true });
    expect(matchesCombo(shiftEvent, plain, true)).toBe(false);
    expect(matchesCombo(plainEvent, shifted, true)).toBe(false);
  });

  it("mod+[ does not swallow ctrl+[ on mac", () => {
    const parsed = parseCombo("mod+bracketleft");
    expect(matchesCombo(ev({ code: "BracketLeft", ctrlKey: true }), parsed, true)).toBe(false);
  });
});

describe("matchesCombo (non-mac)", () => {
  it("mod means Ctrl", () => {
    expect(matchesCombo(ev({ code: "KeyT", ctrlKey: true }), parseCombo("mod+t"), false)).toBe(true);
    expect(matchesCombo(ev({ code: "KeyT", metaKey: true }), parseCombo("mod+t"), false)).toBe(false);
  });

  it("ctrl-only bindings fire with plain Ctrl", () => {
    expect(matchesCombo(ev({ code: "Tab", ctrlKey: true }), parseCombo("ctrl+tab"), false)).toBe(true);
  });
});

describe("dispatchChord", () => {
  // dispatchChord uses the module's own platform detection, so build the mod
  // chord the same way it will be interpreted on the machine running tests.
  const MOD =
    typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "")
      ? { metaKey: true }
      : { ctrlKey: true };

  it("first match wins and returns true", () => {
    const fired: string[] = [];
    const bindings: Binding[] = [
      { combo: "mod+t", handler: () => fired.push("a") },
      { combo: "mod+t", handler: () => fired.push("b") },
    ];
    const hit = dispatchChord(ev({ code: "KeyT", ...MOD }), bindings, false);
    expect(hit).toBe(true);
    expect(fired).toEqual(["a"]);
  });

  it("skips non-allowInInput bindings when an input has focus", () => {
    const fired: string[] = [];
    const bindings: Binding[] = [
      { combo: "mod+t", handler: () => fired.push("blocked") },
      { combo: "mod+t", allowInInput: true, handler: () => fired.push("allowed") },
    ];
    dispatchChord(ev({ code: "KeyT", ...MOD }), bindings, true);
    expect(fired).toEqual(["allowed"]);
  });

  it("returns false when nothing matches", () => {
    expect(dispatchChord(ev({ code: "KeyZ", ...MOD }), [], false)).toBe(false);
  });
});
