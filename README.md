# Vlervcode

A macOS companion app for Claude Code: read-only workspace browser, full-fidelity HTML/Markdown viewer, native file drag-out, and `vlerv://` URL scheme for deep-linking from Claude Code or any other tool.

Built with Tauri 2 + React + TypeScript. Ships as an **11 MB** native `.app` (no Node runtime needed once built).

## Install

```bash
./scripts/build-app.sh
cp -R target/release/bundle/macos/Vlervcode.app /Applications/
```

First launch: right-click → Open (Gatekeeper prompt — the app isn't notarized).

## Use

- **Pick workspace**: first launch shows "Choose workspace folder…" — pick any directory; it's remembered.
- **Browse**: chevron-expand folders inline, click files to preview.
- **Preview**:
  - `.html` → full inline CSS/JS/SVG render (browser fidelity), `<base href>` injected so relative resources resolve.
  - `.md` → marked + shiki + mermaid + katex, centered, dark theme.
  - Code/text → shiki-highlighted.
  - Images → data URI raster / inline SVG (scripts stripped).
- **Drag files out**: drag any file row into Finder, Slack, Mail, Telegram, upload zones — produces a real macOS `kUTTypeFileURL` drop.
- **Switch workspace**: top of sidebar → ⤴ button.

## CLI

```bash
cd cli && cargo build --release
./target/release/vlerv open ~/workspace/some-project/README.md
```

`vlerv open <path>` shells `open vlerv://open?path=<encoded>` — the running app catches the deep-link and opens the file.

## Stack

- Tauri 2 (Rust shell + WKWebView), React 18 + TS + Vite
- Plugins: `tauri-plugin-deep-link`, `tauri-plugin-dialog`, `tauri-plugin-drag`
- Render libs: `marked`, `shiki`, `mermaid`, `katex`

## Develop

```bash
pnpm install
pnpm tauri dev
```

Vite serves the frontend at http://localhost:1420; cargo runs the Tauri shell with hot reload of both the React side and the Rust side.

## Status / next steps

See `STATUS.md` for the current state and open items.
