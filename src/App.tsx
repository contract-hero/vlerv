// Root app — 2-column layout: recursive Explorer sidebar + Preview pane.
import * as React from "react";
import Sidebar from "./components/Sidebar";
import Preview from "./components/Preview";
import { tauriIpc } from "./ipc";
import type { IpcSurface, FilePayload } from "./ipc";
import { useDeepLink } from "./hooks/useDeepLink";
import type { OpenFilePayload, DeepLinkErrorPayload } from "./hooks/useDeepLink";

interface AppProps {
  ipc?: IpcSurface;
}

export default function App({ ipc: injectedIpc }: AppProps = {}): React.ReactElement {
  const ipc = injectedIpc ?? tauriIpc;
  const [selectedFile, setSelectedFile] = React.useState<string | null>(null);
  // True when the current file lies outside the workspace root — either a
  // user-picked ad-hoc file or a vlerv:// deep link that canonicalized
  // outside every configured root. Renders an "external file" badge.
  const [externalFile, setExternalFile] = React.useState<boolean>(false);
  const [payload, setPayload] = React.useState<
    FilePayload | { error: { kind: string; path: string; reason: string } } | null
  >(null);
  const openFileTrigger = React.useRef<(() => void) | null>(null);

  const selectFile = React.useCallback((path: string, external: boolean) => {
    setSelectedFile(path);
    setExternalFile(external);
  }, []);

  const handleDeepLinkIntent = React.useCallback(
    ({ path, intent, out_of_root }: OpenFilePayload) => {
      if (intent === "open") {
        selectFile(path, Boolean(out_of_root));
      } else {
        // Reveal: per deeplink.rs, "selects + expands the tree without
        // switching the preview". Tree expansion needs hoisting of
        // FolderNode.expanded state — tracked as a follow-up. For now we
        // honor the negative half of the contract (don't auto-load preview).
        console.warn("vlerv: reveal intent received; tree-expansion not yet wired", path);
      }
    },
    [selectFile],
  );
  const handleDeepLinkError = React.useCallback(
    ({ reason, url }: DeepLinkErrorPayload) => {
      setPayload({ error: { kind: "DeepLink", path: url, reason } });
    },
    [],
  );
  useDeepLink({ onIntent: handleDeepLinkIntent, onError: handleDeepLinkError });

  // ⌘O / Ctrl+O opens the file picker via the Sidebar trigger.
  React.useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const cmdOrCtrl = e.metaKey || e.ctrlKey;
      if (cmdOrCtrl && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        openFileTrigger.current?.();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // In-iframe link navigation: HTML/Markdown renderers post
  // { type: 'vlerv:navigate', path } when the user clicks a link that
  // resolves to a local file. Route through the same selection path so the
  // file renders in-place and the externalFile flag is recomputed against
  // the current workspace root.
  React.useEffect(() => {
    function isUnderRoot(path: string, root: string | null): boolean {
      if (!root) return false;
      const normalized = root.endsWith("/") ? root : `${root}/`;
      return path === root || path.startsWith(normalized);
    }
    const onMessage = (e: MessageEvent) => {
      const data = e.data as unknown;
      if (!data || typeof data !== "object") return;
      const d = data as { type?: unknown; path?: unknown };
      if (d.type !== "vlerv:navigate" || typeof d.path !== "string") return;
      const root = globalThis.localStorage?.getItem("vlerv.workspaceRoot") ?? null;
      selectFile(d.path, !isUnderRoot(d.path, root));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [selectFile]);

  React.useEffect(() => {
    if (!selectedFile) {
      setPayload(null);
      return;
    }
    let cancelled = false;
    ipc.readFile(selectedFile)
      .then((p) => { if (!cancelled) setPayload(p); })
      .catch((e: Error) => {
        if (!cancelled) {
          setPayload({
            error: { kind: "Io", path: selectedFile, reason: e.message },
          });
        }
      });
    return () => { cancelled = true; };
  }, [selectedFile, ipc]);

  return (
    <div className="app">
      <aside className="pane pane-sidebar" role="complementary">
        <Sidebar
          ipc={ipc}
          onSelectFile={(p, external = false) => selectFile(p, external)}
          selectedFile={selectedFile}
          openFileTrigger={openFileTrigger}
        />
      </aside>
      <main className="pane pane-preview" role="main">
        <Preview
          payload={payload as React.ComponentProps<typeof Preview>["payload"]}
          externalFile={externalFile}
        />
      </main>
    </div>
  );
}
