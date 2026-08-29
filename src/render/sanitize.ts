// Shared sanitizer for markup that lands in the PRIVILEGED host DOM.
//
// Both markdown (raw HTML passthrough) and inline SVG are parsed with
// DOMParser and then importNode'd into the live document, where Tauri IPC is
// in scope and `tauri.conf.json` sets `"csp": null`. DOMParser alone is not
// enough: it neuters <script> only while the tree stays inert, and inline
// `onload=` / `onerror=` handlers and `javascript:` URLs fire the moment the
// nodes go live. Nothing here trusts provenance — a local file gets the same
// treatment as one fetched from a Scope peer or received over Beam, because
// a static document loses nothing by it.
//
// The decisions are pure functions so they are unit-testable without a DOM;
// `sanitizeTree` is the thin DOM walker over them.

/** Elements that execute or re-address, dropped whole (with their subtree).
 *
 * META and LINK are here for the same reason as BASE: they re-address the
 * PRIVILEGED document rather than draw anything. A `<meta http-equiv="refresh"
 * content="0;url=…">` runs its shared declarative refresh steps when the
 * element is inserted into a document — head or body, parsed or importNode'd —
 * so a beamed or Scope-fetched markdown file could navigate the host webview
 * away from the app. `<link rel="stylesheet" href="https://…">` fetches from a
 * machine of the author's choosing. Neither has a legitimate use inside an
 * imported markdown fragment or an inline SVG. `<style>` deliberately stays:
 * a styled SVG is ordinary content and cannot navigate. */
const FORBIDDEN_TAGS = /^(SCRIPT|IFRAME|OBJECT|EMBED|BASE|META|LINK|ANIMATE|SET|ANIMATETRANSFORM)$/;

/** Attributes that can carry a URL — the ones a `javascript:` value arms. */
const URL_ATTRS = new Set(["href", "src", "xlink:href", "srcset", "formaction", "action", "data"]);

export function isForbiddenTag(tagName: string): boolean {
  return FORBIDDEN_TAGS.test(tagName.toUpperCase());
}

/**
 * True for any attribute that can run code once the node is live: every `on*`
 * event handler, and every URL attribute whose value is a `javascript:` URL.
 * Whitespace and control characters are stripped before the scheme test —
 * `java<TAB>script:alert(1)` is a valid URL to the parser.
 */
export function isUnsafeAttribute(name: string, value: string): boolean {
  const lower = name.toLowerCase();
  if (lower.startsWith("on")) return true;
  if (!URL_ATTRS.has(lower)) return false;
  const collapsed = value.replace(/[\s\u0000-\u001f]/g, "").toLowerCase();
  return collapsed.startsWith("javascript:");
}

function scrubAttributes(node: Element): void {
  for (const attr of Array.from(node.attributes)) {
    if (isUnsafeAttribute(attr.name, attr.value)) node.removeAttribute(attr.name);
  }
}

/** Strip forbidden elements and unsafe attributes from a parsed tree, in
 *  place. Call it BEFORE importing the tree into the live document. The root
 *  itself is scrubbed when it is an element — an inline `<svg onload=…>` is
 *  exactly that case, and querySelectorAll never returns its own root. */
export function sanitizeTree(root: ParentNode): void {
  if (root instanceof Element) scrubAttributes(root);
  for (const node of Array.from(root.querySelectorAll("*"))) {
    if (isForbiddenTag(node.tagName)) {
      node.remove();
      continue;
    }
    scrubAttributes(node);
  }
}
