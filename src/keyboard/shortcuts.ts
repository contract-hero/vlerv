// Declarative keyboard-shortcut registry. One capture-phase window keydown
// listener; bindings match on `e.code` (layout-stable for chords) with exact
// modifier matching so "mod+[" never swallows "mod+shift+[".
import * as React from "react";

export interface ChordEvent {
  code: string;
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export interface Binding {
  /** e.g. "mod+t", "mod+shift+bracketright", "ctrl+tab", "mod+digit1". */
  combo: string;
  /** Fire even when an input/textarea/contenteditable has focus. */
  allowInInput?: boolean;
  handler: (e: ChordEvent) => void;
}

interface ParsedCombo {
  mod: boolean;   // ⌘ on macOS (metaKey) or Ctrl elsewhere
  ctrl: boolean;  // explicit Ctrl (for ctrl+tab on macOS)
  shift: boolean;
  alt: boolean;
  code: string;   // normalized e.code
}

const KEY_TO_CODE: Record<string, string> = {
  "[": "BracketLeft",
  "]": "BracketRight",
  "=": "Equal",
  "-": "Minus",
  tab: "Tab",
  enter: "Enter",
  escape: "Escape",
};

function normalizeKeyToken(token: string): string {
  const known = KEY_TO_CODE[token];
  if (known) return known;
  if (/^[a-z]$/.test(token)) return `Key${token.toUpperCase()}`;
  if (/^digit[0-9]$/.test(token)) return `Digit${token.slice(5)}`;
  if (/^[0-9]$/.test(token)) return `Digit${token}`;
  return token; // assume a literal e.code was given
}

export function parseCombo(combo: string): ParsedCombo {
  const parts = combo.toLowerCase().split("+");
  const parsed: ParsedCombo = { mod: false, ctrl: false, shift: false, alt: false, code: "" };
  for (const part of parts) {
    if (part === "mod") parsed.mod = true;
    else if (part === "ctrl") parsed.ctrl = true;
    else if (part === "shift") parsed.shift = true;
    else if (part === "alt") parsed.alt = true;
    else parsed.code = normalizeKeyToken(part);
  }
  return parsed;
}

// Deliberately NOT `guessPlatformOs()` from state/platform: this asks a
// different question. It is true for BOTH macOS and iOS — every Apple
// keyboard puts the primary modifier on ⌘ (metaKey) — where that helper
// tells the two apart. One boolean about the modifier layout, not the shell.
const IS_MAC =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform ?? "");

export function matchesCombo(e: ChordEvent, parsed: ParsedCombo, isMac: boolean = IS_MAC): boolean {
  // Case-insensitive: parseCombo lowercases the whole combo string, so tokens
  // like "bracketleft" reach here lowercased while e.code is "BracketLeft".
  if (e.code.toLowerCase() !== parsed.code.toLowerCase()) return false;
  if (isMac) {
    // Exact match: "mod+[" must not swallow "ctrl+[" and vice versa.
    if (parsed.mod !== e.metaKey) return false;
    if (parsed.ctrl !== e.ctrlKey) return false;
  } else {
    // Non-mac: mod means Ctrl, so a binding needs Ctrl iff it names mod or ctrl.
    if (e.metaKey) return false;
    if ((parsed.mod || parsed.ctrl) !== e.ctrlKey) return false;
  }
  if (parsed.shift !== e.shiftKey) return false;
  if (parsed.alt !== e.altKey) return false;
  return true;
}

function isEditingTarget(): boolean {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  return (
    el.tagName === "INPUT" ||
    el.tagName === "TEXTAREA" ||
    el.isContentEditable
  );
}

/**
 * Dispatch a chord against a binding list. Returns true when a binding fired.
 * Exposed so forwarded iframe chords (`vlerv:keydown` messages) run through
 * the exact same table as native window keydowns.
 */
export function dispatchChord(
  e: ChordEvent,
  bindings: readonly Binding[],
  inInput: boolean,
): boolean {
  for (const binding of bindings) {
    if (inInput && !binding.allowInInput) continue;
    if (matchesCombo(e, parseCombo(binding.combo))) {
      binding.handler(e);
      return true;
    }
  }
  return false;
}

/** Registers the global shortcut listener. Mount exactly once (in App). */
export function useShortcuts(bindings: readonly Binding[]): void {
  const bindingsRef = React.useRef(bindings);
  bindingsRef.current = bindings;

  React.useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const fired = dispatchChord(e, bindingsRef.current, isEditingTarget());
      if (fired) e.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, []);
}
