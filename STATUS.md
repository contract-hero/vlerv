# Vlervcode — current state

## What ships

- **Production `.app` build** via `./scripts/build-app.sh` (~12 MB output).
- **Orcskull icon** baked into the bundle: full Tauri icon set generated from the source webp.
- **Folder picker** on first launch (tauri-plugin-dialog); workspace choice persisted in `localStorage`.
- **Recursive file explorer** with chevron-expanded folders, hover + selection states, 28 px rows.
- **HTML preview**: scripts on, full CSS/JS/SVG, `<base href="file://…">` injected for relative resources. In-iframe `<a>` clicks to local files route back through the host preview via `postMessage`.
- **Markdown preview**: `marked` + lazy `shiki` + lazy `mermaid` + `katex`; centered 900 px max-width.
- **Text/code preview**: shiki for known extensions, plain mono for unknown.
- **Image preview**: data-URI raster, inline SVG with scripts stripped.
- **Metadata card** for oversize / binary fallback.
- **Native file drag-out** via `tauri-plugin-drag` — produces a real macOS `kUTTypeFileURL` drag that Finder, Slack, Mail, and browser upload zones accept.
- **CLI shim** (`cli/src/main.rs`): `vlerv open <path>` and `vlerv reveal <path>` shell to `open vlerv://…`.
- **`vlerv://` URL scheme** declared in `Info.plist` + `tauri.conf.json` (deep-link plugin); the running app catches and dispatches.
- **Foreground on deep link** (PR #18): every `vlerv://` arrival raises + focuses the window from backgrounded / minimized / hidden states.
- **Ad-hoc external file open** (PR #18): ⌘O / "Open File…" picker plus a path input bar (⌘L) accept absolute paths, `file://` URLs, and `vlerv://` URLs anywhere on disk; out-of-workspace files render with an "external file" badge.
- **Auto Light/Dark theme** following macOS system appearance via Tauri's `getCurrentWindow().theme()` + `onThemeChanged()`; iframe background + Shiki code-highlight theme swap with it.
- **Bookmarks**: ⭐ toggle on every file row and in the preview header; collapsible Bookmarks section at the top of the sidebar; persisted across restarts via `state.bookmarks`. Backend emits `vlerv://bookmarks-updated` for cross-pane sync.
- **Resizable sidebar**: 6 px drag handle between the sidebar and preview panes; width persisted to `state.panes.sidebar_px` (clamped 200–480 px).
- **Copy-path button** in the preview header (clipboard API + check-icon feedback).
- **State persistence pipeline wired end-to-end**: `state_store` JSON document at `~/Library/Application Support/Vlerv/state.json`, with `get_state` / `set_state_field` / `list_recents` / `push_recent` / `list_bookmarks` / `add_bookmark` / `remove_bookmark` Tauri commands and matching frontend hooks (`useSettings`, `useRecents`, `useBookmarks`). Recents and Settings now actually persist (previously silent no-op).
- **Filesystem watcher** (PR #17): notify-rs single-root watch with 250 ms debounce, emits `vlerv://tree-changed` for live tree refresh.

## Open items

- `read_file_cmd` (in `lib.rs`, root-gated via `RootSet`) is currently dead code; the live `read_file` command in `main.rs` reads any path. The path bar and ⌘O picker make this surface easier to reach — tighten before letting untrusted content trigger reads.
- `dispatch_deep_link` still hard-errors `EmptyRoots` and `CanonicalizeFailed`; fresh installs with no `~/workspace` dir reject every deep link. Should fall through to `out_of_root: true`.
- Markdown renderer doesn't intercept `<a>` link clicks (HTML iframe does). Relative MD-to-MD links navigate the host webview itself.
- `Settings.tsx`'s `roots[]` array writes through but Explorer ignores it (multi-root tree is half-built; deferred).
- DMG bundling fails inside the AppleScript-driven `bundle_dmg.sh` (Finder permission). The `.app` bundles fine; just `cp -R` to `/Applications/`.
- Tauri build still emits the two pre-existing warnings (`unused_variable: roots_for_handler` in `watcher.rs`; `dead_code: read_file_cmd` in `lib.rs`).
- Frontend has no test runner yet (Vitest not set up). Rust side has 5 unit tests in `dispatch_deep_link_tests`.

## The incident

Earlier development used a multi-agent /forge protocol across 3 cycles. Mid-Cycle-3 the orchestrator ran `rsync -a --delete worker-2/files/ /Users/alilloig/workspace/vlerv-code/` to swap candidates — the `--delete` flag wiped everything in the destination tree (node_modules, all tests, all .forge artifacts, the other worker candidates). 18 source files survived in the staged candidate. This repo is the consolidated recovery.

Git was initialized after the incident; future destructive ops won't escape `git`.
