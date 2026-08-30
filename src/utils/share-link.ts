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
 *  seam the tests substitute. `canShare` matters as much as `share` here —
 *  it is the only way to test a payload without spending the one-shot user
 *  activation that `share` consumes. */
export interface WebShareTarget {
  canShare?(data: { title?: string; text?: string; url?: string }): boolean;
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
 * Share `link` through the Web Share sheet, in exactly ONE `share()` call.
 *
 * `url` is the payload that puts AirDrop in the sheet, so it wins wherever the
 * webview will take it. But `share()` requires transient user activation and
 * CONSUMES it: a second call after a rejection always fails with
 * NotAllowedError, so the choice has to happen BEFORE the first call, not
 * after. `canShare` answers exactly that — it runs the same URL validation and
 * spends nothing. Where `canShare` is missing, `url` is the better guess: it is
 * the payload this feature exists for.
 *
 * A dismissal rejects with AbortError and is not a failure. Anything else
 * reaches the caller, which is what puts it in front of the user.
 */
export async function shareLinkViaWebShare(
  nav: WebShareTarget | undefined,
  link: string,
  title: string,
): Promise<void> {
  const share = nav?.share?.bind(nav);
  if (!share) throw new Error("share-link: Web Share is unavailable in this webview");
  const asUrl = { title, url: link };
  const data = nav?.canShare && !nav.canShare(asUrl) ? { title, text: link } : asUrl;
  try {
    await share(data);
  } catch (err) {
    if (!isShareAbort(err)) throw err;
  }
}
