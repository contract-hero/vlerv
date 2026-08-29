// Markdown renderer — marked + lazy shiki for code highlighting + lazy mermaid.
// Markdown may come from anywhere (address bar, deep links, external tabs),
// so raw HTML passthrough is sanitized before entering the host DOM. The
// parsed HTML is injected via DOMParser → importNode (no innerHTML
// assignment).
import * as React from "react";
import { Marked } from "marked";
import markedKatex from "marked-katex-extension";
import "katex/dist/katex.min.css";
import { useTheme } from "../hooks/useTheme";
import { sanitizeTree } from "./sanitize";

// Module-level Marked instance with the KaTeX extension registered once.
// $inline$ and $$block$$ math both render; throwOnError off so a bad
// expression degrades to red TeX source instead of breaking the document.
const md = new Marked(markedKatex({ throwOnError: false, nonStandard: true }));

export interface MdRendererProps {
  source: string;
  path?: string;
  /** Fired after the markdown DOM lands (before async shiki/mermaid passes)
   *  so the host can restore scroll position. */
  onRendered?: () => void;
}

export default function MdRenderer({ source, path, onRendered }: MdRendererProps): React.ReactElement {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [shikiReady, setShikiReady] = React.useState(false);
  const theme = useTheme();

  // Intercept link clicks. Markdown renders into the host DOM (not a sandboxed
  // iframe like the HTML renderer), so an un-intercepted click would navigate
  // the whole webview away from the app. Both branches funnel through the same
  // postMessage channels App listens on (mirroring html.tsx's intercept), so
  // the open-in-OS-browser policy stays in one place (App + the opener
  // capability):
  //   - http(s)/mailto → `vlerv:openExternal` → OS default browser (Finicky → Chrome)
  //   - file://, absolute, or relative path → `vlerv:navigate` → open in-app.
  // `#`/`javascript:` anchors keep their default in-page behavior.
  const onClickCapture = React.useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const target = e.target as HTMLElement | null;
      const a = target?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!a) return;
      const raw = a.getAttribute("href") ?? "";
      // Only in-page `#` anchors keep their default behavior. Everything else
      // (including `javascript:`) is intercepted — this renders into the host
      // DOM, so an un-prevented `javascript:` href would execute in the
      // privileged webview, not a sandbox.
      if (!raw || raw.startsWith("#")) return;

      if (/^(https?:|mailto:)/i.test(raw)) {
        e.preventDefault();
        window.postMessage({ type: "vlerv:openExternal", url: raw }, "*");
        return;
      }

      const clickMods = {
        meta: e.metaKey || e.ctrlKey,
        shift: e.shiftKey,
        middle: e.button === 1,
      };

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
        window.postMessage({ type: "vlerv:navigate", path: resolved, ...clickMods }, "*");
      }
    },
    [path],
  );

  // Initial render: parse markdown to HTML, then inject via DOMParser+importNode.
  React.useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    let cancelled = false;

    const html = md.parse(source, { gfm: true, breaks: false, async: false }) as string;

    while (el.firstChild) el.removeChild(el.firstChild);
    const doc = new DOMParser().parseFromString(`<div>${html}</div>`, "text/html");
    // Markdown lands in the PRIVILEGED host DOM (Tauri IPC is in scope
    // here), and marked passes raw HTML through unsanitized. DOMParser
    // neuters <script>, but inline `onerror=` handlers and `javascript:`
    // URLs still fire once nodes go live — strip them before importNode
    // below. Same walker the inline-SVG renderer uses.
    sanitizeTree(doc.body);
    const wrapper = doc.body.firstChild;
    if (wrapper) {
      for (const child of Array.from(wrapper.childNodes)) {
        el.appendChild(document.importNode(child, true));
      }
    }

    // Content is in the DOM — let the host restore scroll position.
    onRendered?.();

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
            // The diagram source is artifact-controlled, so the SVG mermaid
            // hands back is too — a label or a node id can carry markup
            // through. Same walker, same reason as the fragment above and as
            // the inline-SVG renderer: this tree is about to go live in the
            // PRIVILEGED host DOM.
            sanitizeTree(parsed);
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
      style={{ width: "100%" }}
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
