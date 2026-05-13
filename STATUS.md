# Vlervcode — current state

## What ships

- **Production `.app` build** via `./scripts/build-app.sh` (~11 MB output).
- **Orcskull icon** baked into the bundle: full Tauri icon set generated from the source webp.
- **Folder picker** on first launch (tauri-plugin-dialog); workspace choice persisted in `localStorage`.
- **Recursive file explorer** with chevron-expanded folders, dark Cursor-style theme, hover + selection states, 28 px rows.
- **HTML preview**: scripts on, full CSS/JS/SVG, `<base href="file://…">` injected for relative resources.
- **Markdown preview**: `marked` + lazy `shiki` + lazy `mermaid` + `katex`; centered 900 px max-width with proper dark-theme typography.
- **Text/code preview**: shiki for known extensions, plain mono for unknown.
- **Image preview**: data-URI raster, inline SVG with scripts stripped.
- **Metadata card** for oversize / binary fallback.
- **Native file drag-out** via `tauri-plugin-drag` — produces a real macOS `kUTTypeFileURL` drag that Finder, Slack, Mail, and browser upload zones accept.
- **CLI shim** (`cli/src/main.rs`): `vlerv open <path>` and `vlerv reveal <path>` shell to `open vlerv://…`.
- **`vlerv://` URL scheme** declared in `Info.plist` + `tauri.conf.json` (deep-link plugin); the running app catches and dispatches.

## Open items

- `read_file_cmd` (in `lib.rs`, root-gated via `RootSet`) is currently dead code; the live `read_file` command in `main.rs` reads any path. Tighten before letting third-party content trigger reads.
- `state_store` / `recents` / `watcher` Rust modules exist but aren't wired to IPC.
- DMG bundling fails inside the AppleScript-driven `bundle_dmg.sh` (Finder permission). The `.app` bundles fine; just `cp -R` to `/Applications/`.
- Tauri build emits two warnings (`unused_variable: roots_for_handler` in `watcher.rs`; `dead_code: read_file_cmd` in `lib.rs`) — both expected for in-flight modules.
- Tests are not yet re-added (lost in the rsync incident; see history).

## The incident

Earlier development used a multi-agent /forge protocol across 3 cycles. Mid-Cycle-3 the orchestrator ran `rsync -a --delete worker-2/files/ /Users/alilloig/workspace/vlerv-code/` to swap candidates — the `--delete` flag wiped everything in the destination tree (node_modules, all tests, all .forge artifacts, the other worker candidates). 18 source files survived in the staged candidate. This repo is the consolidated recovery.

Git was initialized after the incident; future destructive ops won't escape `git`.
