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
  const [payload, setPayload] = React.useState<
    FilePayload | { error: { kind: string; path: string; reason: string } } | null
  >(null);

  const handleDeepLinkIntent = React.useCallback(
    ({ path, intent }: OpenFilePayload) => {
      if (intent === "open") {
        setSelectedFile(path);
      } else {
        // Reveal: per deeplink.rs, "selects + expands the tree without
        // switching the preview". Tree expansion needs hoisting of
        // FolderNode.expanded state — tracked as a follow-up. For now we
        // honor the negative half of the contract (don't auto-load preview).
        console.warn("vlerv: reveal intent received; tree-expansion not yet wired", path);
      }
    },
    [],
  );
  const handleDeepLinkError = React.useCallback(
    ({ reason, url }: DeepLinkErrorPayload) => {
      setPayload({ error: { kind: "DeepLink", path: url, reason } });
    },
    [],
  );
  useDeepLink({ onIntent: handleDeepLinkIntent, onError: handleDeepLinkError });

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
        <Sidebar ipc={ipc} onSelectFile={setSelectedFile} selectedFile={selectedFile} />
      </aside>
      <main className="pane pane-preview" role="main">
        <Preview payload={payload as React.ComponentProps<typeof Preview>["payload"]} />
      </main>
    </div>
  );
}
