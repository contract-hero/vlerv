// Markdown renderer — marked + lazy shiki for code highlighting + lazy mermaid.
// Renders the user's own trusted markdown from their workspace. The parsed
// HTML is injected via DOMParser → importNode (no innerHTML assignment).
import * as React from "react";
import { marked } from "marked";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTheme } from "../hooks/useTheme";

export interface MdRendererProps {
  source: string;
  path?: string;
  onSelectFile?: (path: string) => void;
}

export default function MdRenderer({ source, path }: MdRendererProps): React.ReactElement {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [shikiReady, setShikiReady] = React.useState(false);
  const theme = useTheme();

  // Intercept link clicks. Markdown renders into the host DOM (not a sandboxed
  // iframe like the HTML renderer), so an un-intercepted click would navigate
  // the whole webview away from the app. Instead:
  //   - http(s)/mailto → hand to the OS default browser (Finicky → Chrome here)
  //   - file://, absolute, or relative path → resolve and navigate in-app via
  //     the same `vlerv:navigate` postMessage channel App already listens on.
  // `#`/`javascript:` anchors keep their default in-page behavior.
  const onClickCapture = React.useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const target = e.target as HTMLElement | null;
      const a = target?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!a) return;
      const raw = a.getAttribute("href") ?? "";
      if (!raw || raw.startsWith("#") || raw.startsWith("javascript:")) return;

      if (/^(https?:|mailto:)/i.test(raw)) {
        e.preventDefault();
        void openUrl(raw).catch((err: unknown) => {
          console.error("vlerv: failed to open external URL", raw, err);
        });
        return;
      }

      // Any other scheme would otherwise blow away the host webview — block the
      // default and try to resolve it to a local file we can open in-place.
      e.preventDefault();
      let resolved: string | null = null;
      try {
        if (raw.startsWith("file://")) {
          resolved = decodeURIComponent(new URL(raw).pathname);
        } else if (path) {
          const dir = path.slice(0, path.lastIndexOf("/") + 1);
          const u = new URL(raw, `file://${dir}`);
          if (u.protocol === "file:") resolved = decodeURIComponent(u.pathname);
        }
      } catch {
        resolved = null;
      }
      if (resolved) {
        window.postMessage({ type: "vlerv:navigate", path: resolved }, "*");
      }
    },
    [path],
  );

  // Initial render: parse markdown to HTML, then inject via DOMParser+importNode.
  React.useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    let cancelled = false;

    const html = marked.parse(source, { gfm: true, breaks: false }) as string;

    while (el.firstChild) el.removeChild(el.firstChild);
    const doc = new DOMParser().parseFromString(`<div>${html}</div>`, "text/html");
    const wrapper = doc.body.firstChild;
    if (wrapper) {
      for (const child of Array.from(wrapper.childNodes)) {
        el.appendChild(document.importNode(child, true));
      }
    }

    (async () => {
      // Shiki async pass for code blocks.
      try {
        const { codeToHtml } = await import("shiki");
        const codeBlocks = Array.from(el.querySelectorAll("pre > code"));
        for (const code of codeBlocks) {
          const lang = (code.className.match(/language-(\w+)/)?.[1] ?? "text").toLowerCase();
          if (lang === "mermaid") continue;
          const raw = code.textContent ?? "";
          try {
            const shikiTheme = theme === "light" ? "github-light" : "github-dark";
            const highlighted = await codeToHtml(raw, { lang, theme: shikiTheme });
            if (cancelled) return;
            const replaced = new DOMParser().parseFromString(highlighted, "text/html").body.firstChild;
            if (replaced && code.parentElement) {
              code.parentElement.replaceWith(document.importNode(replaced, true));
            }
          } catch {
            // unknown language — leave raw.
          }
        }
      } catch {
        // shiki not available; leave raw.
      }

      // Mermaid async pass for ```mermaid``` fences.
      try {
        const mermaid = (await import("mermaid")).default;
        mermaid.initialize({ startOnLoad: false, securityLevel: "strict" });
        const nodes = Array.from(el.querySelectorAll('pre > code.language-mermaid'));
        for (let i = 0; i < nodes.length; i++) {
          const code = nodes[i];
          const src = code.textContent ?? "";
          try {
            const { svg } = await mermaid.render(`mermaid-${Date.now()}-${i}`, src);
            if (cancelled) return;
            const wrapper = document.createElement("div");
            wrapper.className = "mermaid";
            const parsed = new DOMParser().parseFromString(svg, "image/svg+xml").documentElement;
            wrapper.appendChild(document.importNode(parsed, true));
            code.parentElement?.replaceWith(wrapper);
          } catch {
            const fallback = document.createElement("pre");
            fallback.setAttribute("data-mermaid-error", "true");
            fallback.textContent = src;
            code.parentElement?.replaceWith(fallback);
          }
        }
      } catch {
        // mermaid not available; leave raw.
      }

      if (!cancelled) setShikiReady(true);
    })();

    return () => { cancelled = true; };
  }, [source, theme]);

  return (
    <div
      data-testid="md-outer"
      style={{ width: "100%", height: "100%", overflow: "auto" }}
      onClickCapture={onClickCapture}
    >
      <div
        ref={containerRef}
        data-shiki-ready={shikiReady ? "true" : "false"}
        data-testid="md-content"
        style={{
          maxWidth: "900px",
          margin: "0 auto",
          padding: "32px 28px",
          lineHeight: 1.6,
        }}
      />
    </div>
  );
}
