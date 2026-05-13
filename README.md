# Vlerv — Claude Code companion app for macOS

Read-only workspace browser + HTML/Markdown viewer with `vlerv://` URL scheme and Finicky integration. **Status: post-incident salvage — see `STATUS.md`.**

## Stack

- **Frontend**: React 18 + TypeScript + Vite (`@tauri-apps/api` for IPC).
- **Renderers**: `marked` + `shiki` + `mermaid` + `katex` for Markdown; sandboxed iframe for HTML; `shiki` for code; inline SVG / data-URI for images.
- **Backend**: Tauri 2 + Rust. Modules:
  - `workspace.rs` — directory scanner with canonical-path cache + log-and-skip per-entry errors.
  - `reader.rs` — file reader with root-anchored security gate + 6 typed error variants.
  - `security.rs` — `RootSet` boundary; `canonicalize_and_check_root` is the load-bearing check.
  - `deeplink.rs` — `vlerv://open?path=…&line=N` and `vlerv://reveal?path=…` parsing.
  - `state_store.rs` — `~/Library/Application Support/Vlerv/state.json` round-trip (unknown-field tolerant).
  - `recents.rs` — last-10 recents with MRU dedup.
  - `watcher.rs` — `notify`-based file watcher emitting Tauri events.
  - `drag_spike.rs` — drag-out payload contract (`public.file-url` + percent-encoded `file://` URL).
- **CLI shim**: `cli/src/main.rs` → builds `vlerv` binary that shells `open vlerv://open?path=…` / `vlerv://reveal?path=…`.

## Run

```bash
pnpm install
pnpm tauri dev
```

First cold build pulls Tauri 2's macOS toolchain (~3 min).

The placeholder icon at `src-tauri/icons/icon.png` is a 1×1 RGBA PNG just so `tauri::generate_context!()` succeeds. Replace with a real iconset before shipping.

## CLI

```bash
cargo build -p vlerv-cli
./target/debug/vlerv open ~/workspace/some-project/README.md
./target/debug/vlerv reveal ~/workspace/some-project/
```

## Finicky integration

Add to `~/workspace/dotfiles/.finicky.js` (inside `handlers: [ … ]`):

```js
{
  match: ({ url }) => url.protocol === 'vlerv:' || url.protocol === 'vlerv',
  browser: 'Vlerv',
},
```

The `vlerv://` scheme is already declared in `src-tauri/Info.plist` (`CFBundleURLTypes`) and `tauri.conf.json` (`plugins.deep-link.desktop.schemes`).

## iTerm2 Semantic History

In iTerm2 → Preferences → Profiles → Advanced → Semantic History, choose "Run command…" and enter:

```
/Users/alilloig/.local/bin/vlerv open \1
```

(Symlink `target/debug/vlerv` to `~/.local/bin/vlerv` first.)
