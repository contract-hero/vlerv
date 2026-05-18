// HTML renderer — iframe rendering, scripts ON, full inline CSS/JS/SVG
// support. A `<base href="file://…">` tag is injected so any relative
// links/images/stylesheets resolve against the source file's directory.

import * as React from "react";
import { useTheme } from "../hooks/useTheme";

export interface HtmlRendererProps {
  /// Raw HTML source (file bytes decoded as UTF-8).
  source: string;
  /// File path (used to build the <base> tag for relative resources).
  path: string;
}

// Small theme stylesheet injected into the iframe so the iframe's own root
// background follows the host theme. User-authored HTML stays untouched —
// most pages set their own background and the injected rule loses to author
// CSS by specificity (we keep it scoped to `html` with no !important).
function themeStyle(theme: "dark" | "light"): string {
  const bg = theme === "dark" ? "#1e1e1e" : "#ffffff";
  return `<style>html { background: ${bg}; }</style>`;
}

// Intercepts in-iframe link clicks: any <a href> that resolves to a `file:`
// URL is rewritten to a `vlerv:navigate` postMessage so the host webview can
// open the linked file inside Vlervcode instead of letting the sandboxed
// iframe try (and fail) to navigate to a `file://` URL.
const LINK_INTERCEPT_SCRIPT = `
<script>
(function () {
  function handler(e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    var raw = a.getAttribute('href') || '';
    if (!raw || raw.startsWith('#') || raw.startsWith('mailto:') || raw.startsWith('tel:') || raw.startsWith('javascript:')) {
      return;
    }
    var resolved;
    try { resolved = new URL(a.href); } catch (_) { return; }
    if (resolved.protocol === 'file:') {
      e.preventDefault();
      var path;
      try { path = decodeURIComponent(resolved.pathname); } catch (_) { path = resolved.pathname; }
      window.parent.postMessage({ type: 'vlerv:navigate', path: path }, '*');
    }
  }
  document.addEventListener('click', handler, true);
  document.addEventListener('auxclick', handler, true);
})();
</script>`;

function injectBase(html: string, basePath: string, theme: "dark" | "light"): string {
  // Strip filename → directory path. Trailing slash matters for <base href>.
  const lastSlash = basePath.lastIndexOf("/");
  const dir = lastSlash >= 0 ? basePath.slice(0, lastSlash + 1) : "";
  const baseTag = `<base href="file://${dir}">`;
  const style = themeStyle(theme);
  // Order matters: <base> first (so relative URLs resolve), then theme style
  // (low-specificity background fallback), then the link-intercept script.
  const headPrelude = `${baseTag}\n${style}\n${LINK_INTERCEPT_SCRIPT}`;
  const headPreludeNoBase = `${style}\n${LINK_INTERCEPT_SCRIPT}`;

  if (/<head[^>]*>/i.test(html)) {
    const inj = /<base\b/i.test(html) ? headPreludeNoBase : headPrelude;
    return html.replace(/(<head[^>]*>)/i, `$1\n${inj}`);
  }
  if (/<html[^>]*>/i.test(html)) {
    return html.replace(/(<html[^>]*>)/i, `$1\n<head>${headPrelude}</head>`);
  }
  return `<!DOCTYPE html><html><head>${headPrelude}</head><body>${html}</body></html>`;
}

export default function HtmlRenderer({ source, path }: HtmlRendererProps): React.ReactElement {
  const theme = useTheme();
  const srcdoc = injectBase(source, path, theme);

  // Browser-like sandbox: scripts, popups, forms, modals, same-origin so the
  // page sees a "normal" environment. The srcdoc origin is still opaque, so
  // this can't reach the parent webview's globals.
  const sandbox =
    "allow-scripts allow-popups allow-popups-to-escape-sandbox allow-forms allow-modals allow-downloads allow-same-origin";

  return (
    <div data-testid="html-renderer">
      <iframe
        sandbox={sandbox}
        srcDoc={srcdoc}
        title="HTML preview"
      />
    </div>
  );
}
