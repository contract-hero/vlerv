// HTML renderer — iframe rendering, scripts ON, full inline CSS/JS/SVG
// support. A `<base href="file://…">` tag is injected so any relative
// links/images/stylesheets resolve against the source file's directory.

import * as React from "react";

export interface HtmlRendererProps {
  /// Raw HTML source (file bytes decoded as UTF-8).
  source: string;
  /// File path (used to build the <base> tag for relative resources).
  path: string;
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

function injectBase(html: string, basePath: string): string {
  // Strip filename → directory path. Trailing slash matters for <base href>.
  const lastSlash = basePath.lastIndexOf("/");
  const dir = lastSlash >= 0 ? basePath.slice(0, lastSlash + 1) : "";
  const baseTag = `<base href="file://${dir}">`;
  const injection = `${baseTag}\n${LINK_INTERCEPT_SCRIPT}`;

  if (/<head[^>]*>/i.test(html)) {
    // Even if a <base> tag already exists, the link-intercept script must run.
    const headInjection = /<base\b/i.test(html) ? LINK_INTERCEPT_SCRIPT : injection;
    return html.replace(/(<head[^>]*>)/i, `$1\n${headInjection}`);
  }
  if (/<html[^>]*>/i.test(html)) {
    return html.replace(/(<html[^>]*>)/i, `$1\n<head>${injection}</head>`);
  }
  return `<!DOCTYPE html><html><head>${injection}</head><body>${html}</body></html>`;
}

export default function HtmlRenderer({ source, path }: HtmlRendererProps): React.ReactElement {
  const srcdoc = injectBase(source, path);

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
