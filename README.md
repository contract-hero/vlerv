# Vlervtifacts

A distraction-free macOS viewer for local HTML artifacts — a native reading room for the reports, plans and pages your tools generate. Browser-style tabs and history over a read-only workspace tree, full-fidelity HTML/Markdown rendering with live reload, native file drag-out, one-link peer-to-peer sharing to another machine (**Beam**), and a `vlerv://` URL scheme for deep-linking from Claude Code or any other tool.

Built with Tauri 2 + React + TypeScript. Ships as a small native `.app` (no Node runtime needed once built).

## Install

```bash
./scripts/build-app.sh
cp -R target/release/bundle/macos/Vlervtifacts.app /Applications/
```

First launch: right-click → Open (Gatekeeper prompt — the app isn't notarized).

## Use

### Tabs & navigation

- **Tabs**: `⌘T` new tab, `⌘W` close, `⌃Tab` / `⌘⇧[` `⌘⇧]` switch, `⌘1–9` jump, drag to reorder. `⌘-click` or middle-click any file row, bookmark, or link inside a preview to open it in a background tab (`⌘⇧-click` foregrounds it).
- **History**: every tab has its own back/forward stack — `⌘[` / `⌘]` or the toolbar arrows.
- **Address bar**: shows the open file's path; type or paste an absolute path / `file://` URL / `vlerv://open?path=…` URL and hit Enter. `⌘L` focuses it, Esc reverts.
- **Reload**: `⌘R` or the toolbar button reloads the current file.
- **Quick open**: `⌘P` fuzzy-searches every file in the workspace.
- **Start page**: an empty tab shows your bookmarks and recent files.

### Live reload

Files open in tabs **auto-reload when they change on disk** — edit an artifact (or let Claude regenerate it) and the preview updates in place, preserving your scroll position. Works for workspace files and for external files open in tabs (their parent dirs are watched individually, surviving atomic saves). Deleted files show a notice and come back automatically if re-created.

### Browsing

- **Pick workspace**: first launch shows "Choose workspace folder…" — pick any directory; it's remembered.
- **Tree**: chevron-expand folders inline; full keyboard navigation (arrows, Enter, Home/End) with VoiceOver-friendly ARIA. The active tab's file auto-reveals in the tree.
- **Right-click** any file row, bookmark, or tab: Open in New Tab, Reveal in Finder, Copy Path, Bookmark. (No "Open in Default App" — that would need an arbitrary-program-launch capability grant; use Reveal in Finder instead.)
- **Bookmarks**: ☆ on a file row or the toolbar star. Drag to reorder, hover ✕ to remove. Persists across restarts.
- **Open any file**: `⌘O` → file picker. Out-of-workspace files render with an "external" badge.
- **Drag files out**: drag any file row into Finder, Slack, Mail, upload zones — a real macOS `kUTTypeFileURL` drop.
- **Zoom**: `⌘+` / `⌘−` / `⌘0`, per tab.
- **Share**: the toolbar Share button opens the native macOS share sheet (AirDrop, Messages, Mail…). With a Slack target configured, an Open-in-Slack button foregrounds that channel — drag the file in from the tree to send it.

### Beam — send an artifact to another machine

**Beam** sends one artifact from your Vlervcode to another — peer-to-peer, end-to-end encrypted, no VPN and no upload. It uses [iroh](https://iroh.computer) for a direct, hole-punched QUIC connection (an encrypted relay is the fallback; content only ever moves over the encrypted link).

- **Send**: the toolbar ⚡ button (or right-click a file → **Beam to Vlervcode…**) stages the file and mints a `vlerv://receive?ticket=…` link. Copy it and send it over any channel you already use — Slack, iMessage, the share sheet. The link is a capability, not the content.
- **Receive**: clicking the link on the other machine raises the app and shows a confirm dialog — file name, size, and the sender's identity fingerprint. **Nothing transfers until you accept**; the stream is integrity-verified (BLAKE3), lands in the app's own `received/` folder, and opens in a tab with a **beamed** badge. Received HTML renders in a hardened, origin-isolated iframe — an artifact authored by someone else can't reach the app.
- **Serving & Stop**: while the app runs it serves the offer (default 24 h, then it expires). The ⚡ indicator lists active offers with fetch counts and a **Stop** button — Stop revokes the link instantly.
- Zero network until you use it: the app opens no sockets until the first beam action.

Both machines need Vlervcode running. First inbound connection may trigger the macOS firewall prompt on the sender; the receiver only dials out.

### Rendering

- `.html` → full inline CSS/JS/SVG render (browser fidelity) in a sandboxed iframe; `<base href>` injected so relative resources resolve. Links to local files navigate in-app (with tab history); `http(s)` links open in the OS browser. **Beamed** (received) HTML renders in a hardened iframe — an opaque origin with no reach into the app and no `<base href>` — since its author is remote and untrusted.
- `.md` → marked + shiki + mermaid + **KaTeX math**, centered, theme-aware.
- Code/text → shiki-highlighted, theme-aware.
- Images → PNG/JPEG/GIF/WebP/BMP/ICO/AVIF render natively (base64 pipeline, 20 MiB cap); SVG renders inline (scripts stripped).

### Chrome

- **Theme**: warm ink (dark) / paper (light), following the macOS system appearance automatically. Artifact iframes and code highlighting swap with it.
- **Native overlay title bar**: the tab strip sits flush with the traffic lights.
- Window size/position, sidebar width, bookmarks and recents all persist across restarts.
- **Deep links foreground the app**: `vlerv://…` arrivals raise + focus the window; if the file is already open its tab is focused instead of duplicating.

## CLI

```bash
cd cli && cargo build --release
./target/release/vlerv open ~/workspace/some-project/README.md
```

`vlerv open <path>` shells `open vlerv://open?path=<encoded>` — the running app catches the deep link and opens the file. `vlerv reveal <path>` expands + highlights it in the tree without switching the preview.

`vlerv beam <path>` opens the send dialog for that file (mint a Beam link); `vlerv receive <ticket>` (or a full `vlerv://receive?…` link) opens the confirm-and-fetch dialog. Both only hand an intent to the app — nothing is staged or fetched without your click.

## Stack

- Tauri 2 (Rust shell + WKWebView), React 18 + TS + Vite
- Plugins: `tauri-plugin-deep-link`, `tauri-plugin-dialog`, `tauri-plugin-drag`, `tauri-plugin-opener`, `tauri-plugin-window-state`
- Render libs: `marked` (+ `marked-katex-extension`), `shiki`, `mermaid`, `katex`
- Beam (P2P transport): `iroh` + `iroh-blobs` (QUIC, hole-punched, content-addressed), exact-version pinned

## Develop

```bash
pnpm install
pnpm tauri dev     # app with hot reload
pnpm test          # vitest (tabs reducer, fuzzy matcher, explorer utils, beam formatting)
cd src-tauri && cargo test   # Rust (watcher, reader, deep links, walk, bookmarks, beam offers/sanitization + a two-endpoint Beam transfer test)
```

## Status / next steps

See `STATUS.md` for the current state and open items.
