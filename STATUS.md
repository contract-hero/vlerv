# Vlerv — current state

## What works

- **Compiles**: `cargo build --workspace` passes (release + debug).
- **TypeScript**: `pnpm exec tsc -p tsconfig.json --noEmit` passes.
- **Real Tauri 2 runtime**: `tauri::Builder` boots with the deep-link plugin and registered IPC handlers (`list_dir`, `list_workspace_roots`).
- **18 survivor source files** (workspace scanner, reader, security boundary, deeplink, drag-spike payload, state store, recents, watcher, all React components and renderers).
- **Reconstructed scaffolding**: Cargo workspace, package.json, tsconfig, vite config, tauri.conf.json, Info.plist, placeholder icon.

## What's missing

1. **All tests** — `src-tauri/tests/*.rs`, `src/__tests__/*.test.tsx`, `tests/` are gone. Cycle 1 had ~36 tests, Cycle 2 had ~88, Cycle 3 had ~144 cumulative. None survived.
2. **`.forge/` artifacts** — `plan.md`, `spec.md` (1246 lines, 17 features, 73 criteria, 48 e2e scenarios), `cycle-plan.md`, all 3 cycle review.md / consolidated.json / synthesis-notes. Reconstructable from this conversation transcript if needed.
3. **C2/C3 features not yet wired to the real Tauri IPC**:
   - `refresh_project` IPC command (Scanner has the methods worker-2 added, but `main.rs` doesn't bind them — quick add).
   - `read_file` IPC command (defined in `lib.rs::read_file_cmd` but not registered in `main.rs`'s `invoke_handler` — currently unused, "dead code" warning).
   - `state_store`/`recents`/`watcher` IPC commands (Rust modules exist; the bindings to Tauri's `invoke_handler` need adding).
4. **Real icon** — current `src-tauri/icons/icon.png` is a 1×1 RGBA placeholder.

## Known soft issues (from C2/C3 reviews before the incident)

- HTML iframe sandbox attribute semantics (test wanted explicit `sandbox=""` form).
- Shiki async timing in MD/text renderers (works in practice; tests needed `waitFor`).
- `Scanner::list_dir` skip-count for chmod-000 dirs may not trigger on stat-succeeds-on-000 macOS path.

## The incident

While trying to swap green-phase worker candidates during Cycle 3, the orchestrator ran:
```
rsync -a --delete worker-2/files/ /Users/alilloig/workspace/vlerv-code/
```
The `--delete` flag removed everything in the destination not in the source candidate — including `.forge/`, `node_modules/`, the other worker candidates, all tests, all config files. No git history existed (the protocol committed only after cycle pass). Git has now been initialized; future destructive ops must be gated by user confirmation.

## How to pick up

```bash
pnpm install        # installs React, Vite, Tauri CLI, marked/shiki/mermaid/katex
cargo build         # warms Tauri 2's macOS deps (~3 min cold)
pnpm tauri dev      # launches Vlerv against http://localhost:1420
```

The window will boot. Click in the sidebar to pick a project, click a file to render.

Next concrete steps (small, well-defined):
1. Register `read_file_cmd` as a Tauri command in `main.rs::invoke_handler`.
2. Wire `state_store`/`recents`/`watcher` modules as IPC commands.
3. Generate a real icon set (`icon.png` at 1024×1024 + `pnpm tauri icon icon.png`).
4. Optional: re-add minimal smoke tests for `workspace::list_dir` and `deeplink::parse` to catch regressions.
