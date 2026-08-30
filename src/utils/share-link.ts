// Sharing a `vlerv://` link (a pairing or beam ticket) is one intent with two
// implementations. macOS pops NSSharingServicePicker through the `share_link`
// command; the iOS companion carries no UIKit bindings, so it goes through
// WKWebView's Web Share API instead. Both decisions live here, away from
// React, so they can be tested without a webview.
import type { IpcSurface, ShareAnchor } from "../ipc";

/** Where the native popover should point. Both share commands take one, so
 *  the rect-to-anchor mapping lives here rather than once per button.
 *
 *  Callers must read this synchronously in the click handler: the rect is
 *  viewport-relative and goes stale the moment the pane behind it scrolls. */
export function shareAnchorFrom(el: Element): ShareAnchor {
  const r = el.getBoundingClientRect();
  return { x: r.x, y: r.y, width: r.width, height: r.height };
}

/** How this build can put a link in a share sheet. `unavailable` is a real
 *  answer: the caller hides the button rather than offering one that does
 *  nothing when tapped. */
export type ShareLinkRoute = "native" | "web-share" | "unavailable";

/** The subset of `navigator` this module uses. Narrow on purpose: it is the
 *  seam the tests substitute. */
export interface WebShareTarget {
  share?(data: { title?: string; text?: string; url?: string }): Promise<void>;
}

export function shareLinkRoute(
  isIos: boolean,
  ipc: Pick<IpcSurface, "shareLink">,
  nav: WebShareTarget | undefined,
): ShareLinkRoute {
  if (isIos) return typeof nav?.share === "function" ? "web-share" : "unavailable";
  return typeof ipc.shareLink === "function" ? "native" : "unavailable";
}

/** The user dismissed the sheet. Nothing failed, so nothing is retried and
 *  nothing is reported. */
export function isShareAbort(err: unknown): boolean {
  return typeof err === "object" && err !== null && (err as { name?: unknown }).name === "AbortError";
}

/**
 * Share `link` through the Web Share sheet.
 *
 * `url` goes first because it is what puts AirDrop in the sheet — the
 * recipient gets something tappable that the `vlerv://` handler opens. WebKit
 * rejects a `url` whose scheme it will not build a URL from, so a rejection
 * retries the same link as text, which is what Messages and Mail transmit
 * anyway. A dismissal is not retried.
 */
export async function shareLinkViaWebShare(
  nav: WebShareTarget | undefined,
  link: string,
  title: string,
): Promise<void> {
  const share = nav?.share?.bind(nav);
  if (!share) throw new Error("share-link: Web Share is unavailable in this webview");
  try {
    await share({ title, url: link });
  } catch (err) {
    if (isShareAbort(err)) return;
    await share({ title, text: link });
  }
}
