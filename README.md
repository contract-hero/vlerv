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
- **Browse**: chevron-expand folders inline, click files to preview. Hover any file row for the ⭐ bookmark toggle.
- **Open any file**: 📄 button or `⌘O` → file picker. Out-of-workspace files render with an "external file" badge.
- **Path bar**: type or paste an absolute path / `file://` URL / `vlerv://open?path=…` URL and hit Enter to navigate. `⌘L` focuses it.
- **Bookmarks**: ⭐ on a file row or in the preview header bookmarks it. Collapsible section at the top of the sidebar holds the list; right-click an entry to remove. Persists across restarts.
- **Resize sidebar**: drag the 6 px gap between the sidebar and the preview pane. Width persists across restarts (clamped 200–480 px).
- **Theme**: follows the macOS system appearance automatically (Light / Dark / Auto in System Settings → Appearance). HTML iframe background and Shiki code-highlight theme swap with it.
- **Foreground on deep link**: `vlerv://…` arrivals raise + focus the window even from a backgrounded / minimized / hidden state.
- **Preview**:
  - `.html` → full inline CSS/JS/SVG render (browser fidelity), `<base href>` injected so relative resources resolve. In-document `<a>` clicks to local files route through the preview pipeline.
  - `.md` → marked + shiki + mermaid + katex, centered, theme-aware.
  - Code/text → shiki-highlighted.
  - Images → data URI raster / inline SVG (scripts stripped).
- **Copy path**: click the copy icon in the preview header — copies the absolute path to the clipboard.
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
