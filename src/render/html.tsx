// HTML renderer — iframe rendering, scripts ON, full inline CSS/JS/SVG
// support. A `<base href="file://…">` tag is injected so any relative
// links/images/stylesheets resolve against the source file's directory.
//
// The injected script speaks a small postMessage protocol with the host:
//   iframe → host:  vlerv:navigate {path, meta, shift, middle}
//                   vlerv:openExternal {url}
//                   vlerv:scroll {x, y}           (rAF-throttled)
//                   vlerv:keydown {code, key, …}  (global-chord forwarding)
//   host → iframe:  vlerv:restoreScroll {x, y}
//                   vlerv:setZoom {zoom}

import * as React from "react";
import { useTheme } from "../hooks/useTheme";
import { useScrollMemory } from "../state/scroll-memory";

export interface HtmlRendererProps {
  /// Raw HTML source (file bytes decoded as UTF-8).
  source: string;
  /// File path (used to build the <base> tag for relative resources).
  path: string;
  /// Content zoom factor (applied to the iframe's documentElement).
  zoom?: number;
  /// Scroll-memory key; when set, scroll position survives reloads and
  /// tab switches.
  scrollKey?: string;
}

// Small theme stylesheet injected into the iframe so the iframe's own root
// background follows the host theme. User-authored HTML stays untouched —
// most pages set their own background and the injected rule loses to author
// CSS by specificity (we keep it scoped to `html` with no !important).
function themeStyle(theme: "dark" | "light"): string {
  const bg = theme === "dark" ? "#1e1e1e" : "#ffffff";
  return `<style>html { background: ${bg}; }</style>`;
}

// Intercepts in-iframe link clicks (the sandboxed iframe can navigate to
// neither file:// nor external http(s)), reports scroll for position memory,
// applies host-driven zoom, and forwards global-shortcut chords the host
// would otherwise never see once the iframe has focus.
const HOST_BRIDGE_SCRIPT = `
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
      window.parent.postMessage({
        type: 'vlerv:navigate',
        path: path,
        meta: !!(e.metaKey || e.ctrlKey),
        shift: !!e.shiftKey,
        middle: e.button === 1
      }, '*');
    } else if (resolved.protocol === 'http:' || resolved.protocol === 'https:') {
      e.preventDefault();
      window.parent.postMessage({ type: 'vlerv:openExternal', url: resolved.href }, '*');
    }
  }
  document.addEventListener('click', handler, true);
  document.addEventListener('auxclick', handler, true);

  // Scroll reporting (rAF-throttled) for host-side position memory.
  var scrollScheduled = false;
  window.addEventListener('scroll', function () {
    if (scrollScheduled) return;
    scrollScheduled = true;
    requestAnimationFrame(function () {
      scrollScheduled = false;
      window.parent.postMessage({ type: 'vlerv:scroll', x: window.scrollX, y: window.scrollY }, '*');
    });
  }, { passive: true });

  // Host-driven scroll restore + zoom.
  window.addEventListener('message', function (e) {
    var d = e.data || {};
    if (d.type === 'vlerv:restoreScroll') {
      window.scrollTo(d.x || 0, d.y || 0);
      // Late layout (images/fonts) can shift content; re-apply once.
      setTimeout(function () { window.scrollTo(d.x || 0, d.y || 0); }, 120);
    } else if (d.type === 'vlerv:setZoom') {
      document.documentElement.style.zoom = String(d.zoom || 1);
    }
  });

  // Forward global-shortcut chords to the host. Only the codes the host
  // actually binds are intercepted, so in-page ⌘C/⌘V/⌘A keep working.
  var FORWARD = {
    KeyT: 1, KeyW: 1, KeyR: 1, KeyL: 1, KeyO: 1, KeyP: 1,
    BracketLeft: 1, BracketRight: 1, Equal: 1, Minus: 1,
    Digit0: 1, Digit1: 1, Digit2: 1, Digit3: 1, Digit4: 1,
    Digit5: 1, Digit6: 1, Digit7: 1, Digit8: 1, Digit9: 1
  };
  window.addEventListener('keydown', function (e) {
    var isChord = e.metaKey || e.ctrlKey;
    if (!isChord) return;
    var forward = FORWARD[e.code] || (e.code === 'Tab' && e.ctrlKey);
    if (!forward) return;
    e.preventDefault();
    window.parent.postMessage({
      type: 'vlerv:keydown',
      code: e.code,
      key: e.key,
      metaKey: e.metaKey,
      ctrlKey: e.ctrlKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey
    }, '*');
  }, true);
})();
</script>`;

function injectBase(html: string, basePath: string, theme: "dark" | "light"): string {
  // Strip filename → directory path. Trailing slash matters for <base href>.
  const lastSlash = basePath.lastIndexOf("/");
  const dir = lastSlash >= 0 ? basePath.slice(0, lastSlash + 1) : "";
  const baseTag = `<base href="file://${dir}">`;
  const style = themeStyle(theme);
  // Order matters: <base> first (so relative URLs resolve), then theme style
  // (low-specificity background fallback), then the host-bridge script.
  const headPrelude = `${baseTag}\n${style}\n${HOST_BRIDGE_SCRIPT}`;
  const headPreludeNoBase = `${style}\n${HOST_BRIDGE_SCRIPT}`;

  if (/<head[^>]*>/i.test(html)) {
    const inj = /<base\b/i.test(html) ? headPreludeNoBase : headPrelude;
    return html.replace(/(<head[^>]*>)/i, `$1\n${inj}`);
  }
  if (/<html[^>]*>/i.test(html)) {
    return html.replace(/(<html[^>]*>)/i, `$1\n<head>${headPrelude}</head>`);
  }
  return `<!DOCTYPE html><html><head>${headPrelude}</head><body>${html}</body></html>`;
}

export default function HtmlRenderer({
  source,
  path,
  zoom = 1,
  scrollKey,
}: HtmlRendererProps): React.ReactElement {
  const theme = useTheme();
  const memory = useScrollMemory();
  const iframeRef = React.useRef<HTMLIFrameElement | null>(null);
  const srcdoc = injectBase(source, path, theme);

  // Track the latest zoom/scrollKey without re-running the load listener.
  const zoomRef = React.useRef(zoom);
  zoomRef.current = zoom;
  const scrollKeyRef = React.useRef(scrollKey);
  scrollKeyRef.current = scrollKey;

  // Save scroll positions reported by THIS iframe (source-filtered — stale
  // iframes from closed tabs can still message during teardown).
  React.useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.source !== iframeRef.current?.contentWindow) return;
      const d = e.data as { type?: unknown; x?: unknown; y?: unknown };
      if (d?.type === "vlerv:scroll" && scrollKeyRef.current) {
        memory.save(scrollKeyRef.current, {
          x: typeof d.x === "number" ? d.x : 0,
          y: typeof d.y === "number" ? d.y : 0,
        });
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [memory]);

  // On every (re)load of the srcdoc: restore scroll + re-apply zoom.
  const onLoad = React.useCallback(() => {
    const win = iframeRef.current?.contentWindow;
    if (!win) return;
    if (zoomRef.current !== 1) {
      win.postMessage({ type: "vlerv:setZoom", zoom: zoomRef.current }, "*");
    }
    const key = scrollKeyRef.current;
    const pos = key ? memory.get(key) : undefined;
    if (pos && (pos.x || pos.y)) {
      win.postMessage({ type: "vlerv:restoreScroll", x: pos.x, y: pos.y }, "*");
    }
  }, [memory]);

  // Live zoom changes (⌘+/−/0 while the page is showing).
  React.useEffect(() => {
    iframeRef.current?.contentWindow?.postMessage({ type: "vlerv:setZoom", zoom }, "*");
  }, [zoom]);

  // Browser-like sandbox: scripts, popups, forms, modals, same-origin so the
  // page sees a "normal" environment. The srcdoc origin is still opaque, so
  // this can't reach the parent webview's globals.
  const sandbox =
    "allow-scripts allow-popups allow-popups-to-escape-sandbox allow-forms allow-modals allow-downloads allow-same-origin";

  return (
    <div data-testid="html-renderer">
      <iframe
        ref={iframeRef}
        sandbox={sandbox}
        srcDoc={srcdoc}
        title="HTML preview"
        onLoad={onLoad}
      />
    </div>
  );
}
