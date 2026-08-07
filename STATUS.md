# Vlervtifacts — current state

## What ships

### Chrome & navigation
- **Browser-style tabs** with per-tab back/forward history (cap 50), pure reducer (`src/state/tabs.ts`, vitest-covered), drag-to-reorder, ⌘T/⌘W/⌃Tab/⌘⇧[]/⌘1-9, middle-click close, "Close Others / Close to the Right" context menu.
- **Toolbar**: sidebar toggle, back/forward/reload, editable address bar (⌘L, accepts absolute / `file://` / `vlerv://` inputs; idle it shows the **workspace-relative** path, focus swaps in the absolute path selected), bookmark star, copy path, external badge, zoom indicator, reader-mode button.
- **View modes**: ⌘B hides/shows the sidebar (persisted, `panes.sidebar_visible`); ⇧⌘F is transient reader mode — sidebar + toolbar gone, tabs + document only; Esc/⇧⌘F/⌘L/⌘B leaves it. Both forwardable from preview iframes. With the sidebar hidden the tab strip reserves the 78px traffic-light gutter.
- **Sidebar sections**: fixed header over three collapsible drawers of one shape (`SidebarSection`, per-drawer localStorage) — Bookmarks (pinned), Recent (last 8 opened, from the recents store), Files (the whole workspace tree, **folded by default**). Retrieval order: Bookmarks → Recent → ⌘P; the tree is for walking a project.
- **Start page** on empty tabs: New York serif wordmark, bookmarks + recents lists, open/pick actions.
- **Quick open (⌘P)**: fuzzy palette over `list_files_recursive` (BFS, 20k cap, ignore/hidden/symlink policy), in-house subsequence scorer, cache invalidated by watcher events.
- **Keyboard registry** (`src/keyboard/shortcuts.ts`): declarative combos, `e.code`-based exact-modifier matching; HTML preview iframes forward tab/nav/zoom chords via `vlerv:keydown` so those survive iframe focus (dialog/focus chords ⌘O/⌘L/⌘P are deliberately not forwardable — page content could synthesize them).
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

### Rendering
- HTML: sandboxed iframe, scripts on, `<base href>` injection, host-bridge script (link intercept with modifiers, scroll report/restore, zoom, chord forwarding).
- Markdown: marked + KaTeX (`marked-katex-extension`, wired for real now) + shiki + mermaid, theme-aware.
- Code/text: **ShikiBlock actually highlights** (was silently broken — created a highlighter and discarded it), theme-aware.
- Images: PNG/JPEG/GIF/WebP/BMP/ICO/AVIF render via backend base64 (`FilePayload.encoding`, 20 MiB cap); SVG inline with scripts stripped.

### Backend (Rust)
- Watcher pipeline refactored (`spawn_pipeline`): shutdown flag + channel-disconnect cascade — **the thread leak per workspace switch is fixed**.
- `RootSet` is Arc-shared; `set_workspace_root` `add_root()`s the picked folder, so deep links into the real workspace classify in-root. `EmptyRoots` falls through to `out_of_root: true` for existing paths (fresh installs accept deep links). `line=N` plumbed into `OpenFileEvent`. `reveal` gets absolute/NUL validation.
- `state_store::flush()` on `RunEvent::Exit` — no more lost writes on fast quit.
- Dead code removed: gated `read_file_cmd`/`read_file_with_roots`/`SecuredFilePayload`, superseded deep-link helpers, `url` crate. `read_file` is deliberately ungated (single-user local viewer; rationale in `reader.rs`).
- `tauri-plugin-window-state`: window geometry persists.

### Design
- Token system in `styles.css` (type/space/radius/motion/elevation/color). Warm **ink** dark theme + **paper** light theme; signature **lamplight amber** accent (active-tab underline, selected-row edge, focus rings, stars, resizer). Native overlay title bar. `:focus-visible` rings, `prefers-reduced-motion` respected.
- Production `.app` build via `./scripts/build-app.sh` (~13 MB `Vlervtifacts.app`).

### Tests
- Rust: 39 (`cargo test` in `src-tauri`) — watcher shutdown/delivery/exact-path/atomic-replace/delete-kind/dedup, reader image + serde wire-shape matrix, recursive walk incl. BFS-truncation invariant, RootSet sharing, deep-link dispatch + recents side-effect matrix, bookmarks.
- Frontend: 59 (`pnpm test`) — tabs reducer (history semantics incl. replace/LOAD_ERROR, tab lifecycle, watcher actions, zoom clamp+quantize), keyboard chord matching/dispatch, address-bar input normalization, click-modifier convention, fuzzy scorer, `ancestorsWithin`.

## Deliberately unchanged (display-only rebrand)

`vlerv://` scheme, `vlerv` CLI binary, bundle id `dev.vlerv.Vlervcode`, state dir `~/Library/Application Support/Vlerv/`, `vlerv.*` localStorage keys, `vlerv://*` event names — external tooling (Finicky, CLAUDE.md deep-link instructions) depends on the scheme; the rest avoids a pointless migration. Delete the old `Vlervcode.app` from `/Applications` after installing so LaunchServices doesn't route `vlerv://` to the stale binary.

## Open items

- Deep-link `line=N` reaches the frontend but no renderer scrolls to a line yet.
- Recents list is push-only from opens; no backend broadcast event (StartPage refreshes on mount).
- `preferences.ignore_globs` / `drag_out_mode` still unwired (the hardcoded `DEFAULT_IGNORED` covers the real use). `Settings.tsx` exists and holds the Slack-target field but is still not mounted anywhere — set `preferences.slack_target` via state.json until it is (product decision deferred in #22).
- Markdown auto-reload re-runs mermaid/KaTeX from scratch — a large doc may flash briefly on reload.
- DMG bundling still fails in `bundle_dmg.sh` (Finder permission); `.app` bundles fine, `cp -R` to `/Applications/`.

## History

Earlier development lost most of the repo to an `rsync --delete` incident (see git history); this codebase is the consolidated recovery, since rebuilt: rebrand to Vlervtifacts + tabs/history/live-reload architecture + visual identity (July 2026).
