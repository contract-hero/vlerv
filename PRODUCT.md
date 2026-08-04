# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The frontend is React + TypeScript rendered in WKWebView inside a Tauri 2 macOS
shell. It is not an iOS or Android product. macOS desktop affordances are part
of the design surface (see Operating Context).

## Users

One user: the repository owner, a Sui/Move engineer who runs Claude Code and
other agentic tools all day across a ~100-repo workspace at `~/workspace/`.

The job: those tools emit HTML reports, audits, plans, explainers and Markdown
notes to disk. The user must read them at the moment they are produced, jump
between several of them, watch them change as an agent regenerates them, and
send one to a colleague. A browser makes this work feel like tab archaeology.
A code editor renders the source, not the artifact.

Confirmed scope: personal tool. No second audience, no onboarding for
strangers, no marketing surface. Design decisions optimize for one expert user
who already knows the product.

## Product Purpose

Vlervtifacts is a distraction-free macOS reading room for local HTML and
Markdown artifacts.

It succeeds when the user opens a generated artifact in one action, reads it at
full browser fidelity, and never manages files, tabs, or reloads by hand.
It fails when the user goes back to a browser or a file manager to do
something Vlervtifacts should have done.

## Positioning

Three properties together, which no neighboring tool has:

1. **Deep-link addressable.** The `vlerv://open?path=…` URL scheme makes any
   agent, script, or Markdown link able to open a specific local file, at a
   specific line, in this app. Claude Code sessions end with a clickable
   `vlerv://` link by user convention.
2. **Live reload on local files.** A file open in a tab reloads when it changes
   on disk, and keeps its scroll position. The user watches an agent rewrite a
   report in place.
3. **Read-only workspace tree with browser ergonomics.** Tabs, per-tab
   back/forward history, quick open, bookmarks and recents over a directory —
   without the risk surface of an editor.

A browser has no workspace tree, no live reload for `file://`, and no deep-link
scheme. An editor renders source. A Markdown viewer has no HTML fidelity.

## Operating Context

- **macOS only, permanently.** Confirmed durable constraint. Design may depend
  on macOS system affordances: traffic lights under an overlay title bar,
  system light/dark appearance, native share sheet, Reveal in Finder, native
  file drag-out (`kUTTypeFileURL`), LaunchServices URL-scheme routing.
- **Trigger sources.** Files arrive from Claude Code sessions, the `vlerv` CLI
  binary, a `⌘O` file picker, the workspace tree, and pasted paths.
- **Companion surfaces.** A `vlerv` CLI (`vlerv open`, `vlerv reveal`) and a
  Slack hand-off path (native share sheet plus an Open-in-Slack deep link).
- **Install shape.** A local, unnotarized `.app` built by
  `./scripts/build-app.sh` and copied to `/Applications`. First launch needs a
  right-click → Open past Gatekeeper.
- **Distribution.** No release channel, no auto-update, no telemetry.

## Capabilities and Constraints

**Shipped capabilities.** Browser-style tabs with per-tab history; editable
address bar accepting absolute paths, `file://` and `vlerv://`; quick open
(`⌘P`) with a fuzzy matcher; bookmarks and recents that persist; a keyboard
registry with exact-modifier chords, forwarded into preview iframes; per-tab
zoom; workspace tree with full keyboard navigation and ARIA; context menus;
live reload with scroll restoration; a start page on empty tabs; a settings
surface that exists but is not mounted.

**Rendering.** HTML in a sandboxed iframe with injected `<base href>` and a
host bridge. Markdown through marked, KaTeX, shiki and mermaid. Code and text
through shiki. Images through a base64 backend pipeline with a 20 MiB cap.
Inline SVG with scripts stripped.

**Stack.** Tauri 2 (Rust + WKWebView), React 18, TypeScript, Vite, pnpm.
Plugins: deep-link, dialog, drag, opener, window-state. Tests: vitest for the
frontend, `cargo test` for Rust.

**Durable technical constraints.**

- The webview is a privileged surface. Artifact content is untrusted: it runs
  in a sandboxed iframe, and capability grants stay narrow on purpose.
  "Open in Default App" was rejected because it needs an
  arbitrary-program-launch grant reachable from webview IPC.
- Identifiers are frozen for compatibility, not for taste: the `vlerv://`
  scheme, the `vlerv` CLI name, bundle id `dev.vlerv.Vlervcode`, the state
  directory `~/Library/Application Support/Vlerv/`, `vlerv.*` localStorage
  keys, and `vlerv://*` event names. External tooling depends on the scheme.
- The workspace tree is read-only. The product reads and reveals files. It
  does not create, rename, or delete them.

**Open product decisions.** Where the Settings surface belongs, and whether it
ships at all. Whether deep-link `line=N` should scroll a renderer to that line
(it reaches the frontend and stops there). Whether recents need a backend
broadcast. Whether `ignore_globs` and `drag_out_mode` preferences become real.
DMG bundling currently fails; `.app` is the only bundle target.

## Brand Commitments

Binding: the product name **Vlervtifacts**, the `vlerv://` URL scheme, and the
`vlerv` CLI name.

Not binding: everything visual. The incumbent identity — warm ink and paper
neutrals, a lamplight amber accent, a New York serif wordmark, "the chrome
recedes, the artifact is the hero" — is confirmed as replaceable. A later
redesign may choose a different visual world.

## Evidence on Hand

- `README.md` and `STATUS.md`: accurate, current feature and status records.
- A mature incumbent design system in `src/styles.css` (~1050 lines of tokens
  and rules) with dark and light themes.
- App icons in `src-tauri/icons/`.
- Real artifacts to test against: HTML reports, audits and explainers across
  `~/workspace/`.
- Absent, and not to be fabricated: users, testimonials, download counts,
  benchmarks, pricing, licensing claims beyond the repository `LICENSE`, and
  any release or notarization status.

## Product Principles

1. **The artifact is the content; the app is furniture.** Every element of
   chrome must earn the pixels it takes from the artifact.
2. **One action from anywhere to the file.** A deep link, a CLI call, a path
   paste, and a tree click all land in the same place with the same result.
3. **The file on disk is the truth.** The view follows the file, keeps the
   reader's place, and never asks the user to reload.
4. **Read, do not edit.** The product's power comes from what it refuses to
   touch, including the capability grants it declines to request.
5. **Built for one expert.** Prefer keyboard depth and density over
   discoverability aids for users who do not exist.

## Accessibility & Inclusion

No external standard is required. The incumbent build already carries
VoiceOver-friendly `role=tree` semantics, `aria-expanded`/`aria-level`, roving
tabindex, `:focus-visible` rings, and `prefers-reduced-motion` support. Treat
these as a floor to preserve, not a target to reach.
