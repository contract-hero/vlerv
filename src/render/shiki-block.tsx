// Async-highlighted code block — wraps shiki's codeToHtml. Sets
// `data-shiki-ready="true"` on the wrapper once highlighting resolves.
// Theme-aware: follows the host light/dark theme like md.tsx does.
import * as React from "react";
import { useTheme } from "../hooks/useTheme";

export interface ShikiBlockProps {
  code: string;
  lang: string;
}

export default function ShikiBlock({ code, lang }: ShikiBlockProps): React.ReactElement {
  const theme = useTheme();
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const [ready, setReady] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    setReady(false);
    import("shiki")
      .then(async ({ codeToHtml }) => {
        const shikiTheme = theme === "light" ? "github-light" : "github-dark";
        const html = await codeToHtml(code, { lang, theme: shikiTheme });
        if (cancelled) return;
        const el = containerRef.current;
        if (el) {
          const doc = new DOMParser().parseFromString(html, "text/html");
          const pre = doc.body.firstChild;
          while (el.firstChild) el.removeChild(el.firstChild);
          if (pre) el.appendChild(document.importNode(pre, true));
        }
        setReady(true);
      })
      .catch(() => {
        // Unknown language or shiki unavailable — keep the plain fallback.
        if (!cancelled) setReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, [code, lang, theme]);

  return (
    <div data-shiki-block data-shiki-ready={ready ? "true" : undefined} className="shiki-block">
      {/* Plain fallback shown until (or in place of) the highlighted DOM. */}
      <div ref={containerRef}>
        <pre>
          <code className={`language-${lang}`}>{code}</code>
        </pre>
      </div>
    </div>
  );
}
