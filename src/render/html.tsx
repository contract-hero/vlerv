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

function injectBase(html: string, basePath: string): string {
  // Strip filename → directory path. Trailing slash matters for <base href>.
  const lastSlash = basePath.lastIndexOf("/");
  const dir = lastSlash >= 0 ? basePath.slice(0, lastSlash + 1) : "";
  const baseTag = `<base href="file://${dir}">`;

  if (/<base\b/i.test(html)) {
    return html;
  }
  if (/<head[^>]*>/i.test(html)) {
    return html.replace(/(<head[^>]*>)/i, `$1\n${baseTag}`);
  }
  if (/<html[^>]*>/i.test(html)) {
    return html.replace(/(<html[^>]*>)/i, `$1\n<head>${baseTag}</head>`);
  }
  return `<!DOCTYPE html><html><head>${baseTag}</head><body>${html}</body></html>`;
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
