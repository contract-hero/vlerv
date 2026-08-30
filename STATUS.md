# Vlervtifacts — current state

## What ships

### Chrome & navigation
- **Browser-style tabs** with per-tab back/forward history (cap 50), pure reducer (`src/state/tabs.ts`, vitest-covered), drag-to-reorder, ⌘T/⌘W/⌃Tab/⌘⇧[]/⌘1-9, middle-click close, "Close Others / Close to the Right" context menu.
- **Toolbar**: sidebar toggle, back/forward/reload, editable address bar (⌘L, accepts absolute / `file://` / `vlerv://` inputs; idle it shows the **workspace-relative** path, focus swaps in the absolute path selected), bookmark star, copy path, external badge, zoom indicator, reader-mode button.
- **View modes**: ⌘B hides/shows the sidebar (persisted, `panes.sidebar_visible`); ⇧⌘F is transient reader mode — sidebar + toolbar gone, tabs + document only; Esc/⇧⌘F/⌘L/⌘B leaves it. Both forwardable from preview iframes. With the sidebar hidden the tab strip reserves the 78px traffic-light gutter.
- **Sidebar sections**: fixed header over three collapsible drawers of one shape (`SidebarSection`, per-drawer localStorage) — Bookmarks (pinned), Recent (last 8 opened, from the recents store), Files (the whole workspace tree, **folded by default**). Retrieval order: Bookmarks → Recent → ⌘P; the tree is for walking a project.
- **Start page** on empty tabs: New York serif wordmark, bookmarks + recents lists, open/pick actions.
- **Quick open (⌘P)**: fuzzy palette over `list_files_recursive` (BFS, 20k cap, ignore/hidden/symlink policy), in-house subsequence scorer, cache invalidated by watcher events.
- **Keyboard registry** (`src/keyboard/shortcuts.ts`): declarative combos, `e.code`-based exact-modifier matching; HTML preview iframes forward tab/nav/zoom/view-mode chords plus bare Escape (bound only in reader mode) via `vlerv:keydown` so those survive iframe focus (dialog/focus chords ⌘O/⌘L/⌘P are deliberately not forwardable — page content could synthesize them).
- **Per-tab zoom** (⌘+/−/0) via CSS `zoom` — host content directly, iframes via `vlerv:setZoom`.

### Live reload (the headline feature)
- Workspace watcher events + dedicated `vlerv://file-changed` events fan out through a subscription bus (`src/state/watcher-bus.tsx`) to (a) surgical Explorer folder invalidation and (b) tab auto-reload (150 ms/path debounce). Open out-of-root files are individually watched via `watch_external_paths` (parent-dir NonRecursive watches → survives atomic rename saves).
- **Scroll position survives reloads and tab switches**: iframe `vlerv:scroll`/`vlerv:restoreScroll` protocol (source-filtered), container scrollTop for md/text, `ScrollMemoryProvider` keyed `tabId::path`.
- `remove` events show a "File deleted" notice (tab kept); re-add auto-recovers.

### Explorer
- Hoisted expansion state (`ExplorerUiProvider`); `reveal(path)` expands ancestor chains in one update — the deep-link `reveal` intent works, and the active tab's file auto-reveals.
- Tree keyboard navigation (arrows/Enter/Home/End), `role=tree/treeitem`, `aria-expanded/level`, roving tabindex.
- Context menus everywhere (custom, no dep): Open in New Tab, Reveal in Finder, Copy Path, Bookmark toggle.
- **Share** (merged from #22): native macOS share sheet + Open-in-Slack live in the Toolbar next to the star/copy buttons (they were in the removed Preview header). ("Open in Default App" was dropped in review: it required `opener:allow-open-path **`, an arbitrary-program-launch grant reachable from webview IPC.)

### Beam (remote-control-design.html — v1)
- **P2P artifact transfer between Vlervcode instances** over [iroh](https://iroh.computer) (QUIC, hole-punched, encrypted relay fallback; exact-version pinned, all iroh types quarantined in `crates/vlerv-remote/`, behind the `src-tauri/src/remote.rs` facade). Lazy boot: zero sockets until the first beam action.
- **Send**: toolbar ⚡ button (or `vlerv beam <path>`, or a `vlerv://beam?path=…` link — link-initiated sends show a confirm face first). Stages the file into a content-addressed store (`ImportMode::Copy` — the ticket pins bytes at mint time), mints a `vlerv://receive?ticket=…&name=…&size=…` link. Path policy = the share module's (`canonicalize_allow_external`, conservative on empty roots).
- **Serve**: a per-request gate (`iroh-blobs` provider events) intercepts every request kind and admits only a plain full-blob GET whose hash is an active, unexpired offer — get-many / push / observe are refused explicitly. **Stop and TTL expiry revoke the in-memory offer instantly** (the staged bytes are unpinned but linger until blob GC lands — see Open items). Fetch counts come from the same gate. Offers live in the toolbar ⚡ indicator (name, fetches, expiry, Stop) with a Received section.
- **Receive**: deep link → confirm dialog (sanitized name — bidi/format chars stripped, size claim, sender NodeId fingerprint) → BLAKE3-verified stream (256 MiB hard cap enforced on actual bytes, 20 MiB warn) → lands under `Application Support/Vlerv/received/<date>/`, opens in a tab with a **beamed** badge. "Sender offline" is retryable from the dialog.
- **Untrusted-content isolation**: a beamed artifact is authored by the sender, not the local user, so received HTML renders in a hardened iframe — the sandbox drops `allow-same-origin` (opaque origin, no reach into the host webview / Tauri IPC) and no `file://` base-href is injected (`HtmlRenderer.isolate`, keyed on the received/ prefix). Local files render unchanged.
- **Identity**: ed25519 keypair persisted 0600 at `remote/identity.key`; corrupt key = hard error (silent regen would orphan shared tickets).
- Preferences: `preferences.beam_ttl_hours` (default 24, settable via state.json until Settings mounts).
- **Binary-size impact (measured, per design §8)**: release `vlerv-app` 12 MB → 28 MB with `iroh` + `iroh-blobs`. Larger than the design's "several-MB" guess; accepted for a local .app, revisit only if it starts to hurt.

### Scope (remote-control-design.html — v2)
- **Pairing**: `vlerv://pair?ticket=…` deep link (or QR-scannable equivalent) mints a one-time pairing token; both sides confirm a **six-word fingerprint** derived from both NodeIds before the peer is persisted. Peers live in `remote/peers.json` (NodeId, device name, granted scope, paired-at, last-seen); revocation is deleting the entry.
- **Getting the invite to the other device**: Settings offers Copy link *and* Share. Share opens the native sheet — AirDrop, Messages, Mail — with the link as a URL, so the recipient taps it and the `vlerv://` handler opens it. macOS routes through the `share_link` command (NSSharingServicePicker; `share.rs` admits a link only if `deeplink::parse` accepts it as `Pair`/`Receive`, so the sheet cannot carry a link the recipient's app would refuse, nor a path-carrying verb). iOS has no UIKit bindings in this build and uses the WKWebView Web Share API (`src/utils/share-link.ts`). **One `share()` call per click**: it requires transient user activation and consumes it, so a retry after a rejection always fails with `NotAllowedError` — `canShare()` picks URL-vs-text *before* the call instead. A failed share flips the button to "Share failed" rather than doing nothing. Where neither route exists the button does not render.
- **Fingerprint dialog stacking**: `RemotePairDialog` sits above the Settings surface (`.beam-backdrop` z-index 150 vs `.settings-backdrop` 100). A pair link normally arrives while Settings is open, and six words that a dialog covers cannot be compared.
- **Scope server/client** under ALPN `vlerv/scope/0`, multiplexed on the same iroh endpoint as Beam's blob ALPN. Scopes: `view-open`, `browse`, `control`.
- **Remote sidebar drawers**: a paired, online peer appears as a fourth drawer beside Bookmarks/Recent/Files — live tab list, lazy workspace tree under `browse`.
- **Live-follow**: a drawer toggle mirrors the host's active tab as it switches.
- **`control` scope / `OpenOnHost`**: a `control`-scoped peer can push-open an artifact on the host — the literal remote control.
- Known gap: `remote_set_scope` narrowing an existing peer's scope does not revoke grants already minted under the wider scope — those stay valid for up to 1 h.

### MCP server (crates/vlerv-mcp)
- Stdio MCP server named `vlerv-mcp` (rmcp 3.1.4) that exposes Remote Control to external agents: `beam_artifact`, `list_devices`, `send_to_device`, `pair_device`, `pair_status`, `confirm_pairing`, `stop_beam`, `server_status`.
- Configured via env: `VLERV_MCP_ROOTS`, `VLERV_MCP_STATE_DIR`, `VLERV_STATE_DIR`.
- Built on the Tauri-free `crates/vlerv-remote` core, so the MCP binary carries no Tauri dependency.
- Documented in `README-MCP.md`.
- Known gap: no per-peer push quota on `send_to_device` / `beam_artifact`.

### iOS companion
- Tauri iOS target builds and runs on the iPhone simulator (`scripts/build-ios-sim.sh`).
- Receive-focused UI: `IosStartPage` ("Pair with a Mac"), `ReceivedDrawer`, remote drawers; macOS-only code is cfg-gated out of the iOS build.
- Debug-only `VLERV_TEST_AUTOPAIR` E2E hook in `src-tauri` (three arms) — confirmed absent from release builds by running `strings` on the release binary.
- **Live E2E succeeded (2026-08-29)**: the MCP server paired with the simulator app via a `vlerv://pair` deep link and pushed an artifact that opened on the phone.
- **Phone-adapted layout** (`PhoneShell.tsx`): on iOS the desktop panes never mount. One column — a 40px title band (active artifact's basename + a live-reload pulse dot in the accent), the artifact full-bleed, and a bottom bar at the thumb (Library, back/forward, tab count). Library (Remote + Received + Settings) and the open-tab list are bottom sheets over a scrim; safe-area insets respected, `100dvh` instead of `100vh` (WKWebView's vh overshoots the visible viewport and hid the bar). Same design tokens as the desktop — the phone changes the architecture, not the language.
- Theme: Tauri's `window.theme()` reports "light" on iOS regardless of the trait collection, so `useTheme` skips the Tauri probe on iOS (UA check — the `platform-ios` body class arrives async and would race it) and trusts `prefers-color-scheme`, which WKWebView tracks correctly.
- Known gap: device name shows the Mac hostname on the simulator, not a real iOS device name.

### Rendering
- HTML: sandboxed iframe, scripts on, `<base href>` injection, host-bridge script (link intercept with modifiers, scroll report/restore, zoom, chord forwarding).
- Markdown: marked + KaTeX (`marked-katex-extension`, wired for real now) + shiki + mermaid, theme-aware.
- Code/text: **ShikiBlock actually highlights** (was silently broken — created a highlighter and discarded it), theme-aware.
- Images: PNG/JPEG/GIF/WebP/BMP/ICO/AVIF render via backend base64 (`FilePayload.encoding`, 20 MiB cap); SVG inline with scripts stripped.

### Backend (Rust)
- Workspace reorganized: the shared Remote Control core — security `RootSet`, `proto`, `peers`, `endpoint`, `beam`, `scope` — moved into a Tauri-free crate, `crates/vlerv-remote`, with `EventSink`/`HostCatalog`/`Dirs` seams. Both `src-tauri` and the new `crates/vlerv-mcp` binary consume it; neither pulls in Tauri from the other.
- Watcher pipeline refactored (`spawn_pipeline`): shutdown flag + channel-disconnect cascade — **the thread leak per workspace switch is fixed**.
- `RootSet` is Arc-shared; `set_workspace_root` `add_root()`s the picked folder, so deep links into the real workspace classify in-root. `EmptyRoots` falls through to `out_of_root: true` for existing paths (fresh installs accept deep links). `line=N` plumbed into `OpenFileEvent`. `reveal` gets absolute/NUL validation.
- `state_store::flush()` on `RunEvent::Exit` — no more lost writes on fast quit.
- Dead code removed: gated `read_file_cmd`/`read_file_with_roots`/`SecuredFilePayload`, superseded deep-link helpers, `url` crate. `read_file` is deliberately ungated (single-user local viewer; rationale in `reader.rs`).
- `tauri-plugin-window-state`: window geometry persists.

### Design
- Token system in `styles.css` (type/space/radius/motion/elevation/color). Warm **ink** dark theme + **paper** light theme; signature **lamplight amber** accent (active-tab underline, selected-row edge, focus rings, stars, resizer). Native overlay title bar. `:focus-visible` rings, `prefers-reduced-motion` respected.
- Production `.app` build via `./scripts/build-app.sh` (~28 MB `Vlervtifacts.app` since Beam pulled in iroh — was ~13 MB).

### Tests
- Rust: ~186 tests green across the cargo workspace (`src-tauri`, `crates/vlerv-remote`, `crates/vlerv-mcp`) — `src-tauri` covers watcher shutdown/delivery/exact-path/atomic-replace/delete-kind/dedup, reader image + serde wire-shape matrix, recursive walk incl. BFS-truncation invariant, RootSet sharing, deep-link dispatch + recents side-effect matrix (incl. beam/receive/pair verbs, hostile name-hint sanitization incl. bidi/format chars, ticket rejection), bookmarks, offers registry admit/expiry/revocation, `resolve_offerable` gate/file/cap, TTL clamp, identity persistence; `crates/vlerv-remote` covers security/peers/scope/beam/endpoint standalone from Tauri; `crates/vlerv-mcp` covers its tool handlers. `src-tauri` still carries the **two-endpoint in-process Beam round trip** integration test (offer → link re-parse → gated fetch → verified landing → collision naming → Stop → denial → no orphaned partial); the data path is loopback (the receiver dials a 127.0.0.1 re-mint of the ticket), though the endpoints still boot the n0 preset, so `offer()` waits up to 10 s on `online()` when relays are unreachable.
- Frontend: 131 green (`pnpm test`) — tabs reducer (history semantics incl. replace/LOAD_ERROR, tab lifecycle, watcher actions, zoom clamp+quantize), keyboard chord matching/dispatch, address-bar input normalization, click-modifier convention, fuzzy scorer, `ancestorsWithin`, beam formatting helpers, Scope/remote-drawer state. `tsc` is clean.

## Deliberately unchanged (display-only rebrand)

`vlerv://` scheme, `vlerv` CLI binary, bundle id `dev.vlerv.Vlervcode`, state dir `~/Library/Application Support/Vlerv/`, `vlerv.*` localStorage keys, `vlerv://*` event names — external tooling (Finicky, CLAUDE.md deep-link instructions) depends on the scheme; the rest avoids a pointless migration. Delete the old `Vlervcode.app` from `/Applications` after installing so LaunchServices doesn't route `vlerv://` to the stale binary.

## Open items

- **Beam follow-ups** (remote-control-design.html M6): blob-store GC for stopped/expired offers (tags are deleted; bytes linger in `remote/blobs/` until a GC pass exists), single-fetch mode, lock-to-peer beams, Open-in-Slack for the beam *link* and wiring `ShareLinkButton` into `BeamDialog` (the `share_link` command ships and the pairing invite already uses it; BeamDialog is still Copy-link only), Settings UI for `beam_ttl_hours`, macOS application-firewall prompt doc for unnotarized inbound. `endpoint.online()` waits up to 10 s before minting when relays are unreachable (e.g. behind some VPNs) — the ticket still carries direct addrs.
- **Scope follow-ups**: `remote_set_scope` narrowing an existing peer's scope does not revoke grants already minted under the wider scope (valid up to 1 h after narrowing); no per-peer push quota on MCP/Scope pushes.
- **iOS companion follow-ups**: device name shows the Mac hostname on the simulator instead of a real iOS device name. **Container-relative persistence**: received entries, tab history and `peers.json` paths persist as absolute container paths, but iOS moves the app container UUID on every app update — persisted tabs then 404 and pairing state is orphaned (observed on simulator reinstall). Persist paths relative to the Application Support root and re-resolve at load.
- Deep-link `line=N` reaches the frontend but no renderer scrolls to a line yet.
- Recents list is push-only from opens; no backend broadcast event (StartPage refreshes on mount).
- `preferences.ignore_globs` / `drag_out_mode` still unwired at the backend (the hardcoded `DEFAULT_IGNORED` covers the real use), though `Settings.tsx` now edits both. Settings is mounted on both platforms: a centered dialog from the sidebar gear on macOS, a bottom sheet from the Library sheet on iOS. The phone owns no files (read-only companion), so it shows the Remote section only — roots, ignore set, drag-out and the Slack target are desktop-only.
- Markdown auto-reload re-runs mermaid/KaTeX from scratch — a large doc may flash briefly on reload.
- DMG bundling still fails in `bundle_dmg.sh` (Finder permission); `.app` bundles fine, `cp -R` to `/Applications/`.

## History

Earlier development lost most of the repo to an `rsync --delete` incident (see git history); this codebase is the consolidated recovery, since rebuilt: rebrand to Vlervtifacts + tabs/history/live-reload architecture + visual identity (July 2026).
