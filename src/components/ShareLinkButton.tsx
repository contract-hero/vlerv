// ShareLinkButton — the companion to CopyLinkButton. Copy puts the link on
// THIS machine's clipboard, but a pairing link's whole job is to reach the
// OTHER device: the share sheet does that in one step (AirDrop, Messages,
// Mail) instead of a clipboard plus a second app.
//
// Rendering nothing when no route exists is deliberate — a share button that
// silently does nothing on tap is worse than no button (see `shareLinkRoute`).
// For the same reason a failed share is reported rather than swallowed: the
// OS sheet only speaks for itself once it OPENS, and every failure worth
// naming happens before that.
import * as React from "react";
import { Share } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { usePlatform } from "../state/platform";
import { shareAnchorFrom, shareLinkRoute, shareLinkViaWebShare } from "../utils/share-link";

/** Longer than useCopyFeedback's 1200ms: a failure has to be read, not just
 *  noticed. */
const FAIL_RESET_MS = 2400;

export default function ShareLinkButton({
  link,
  sheetTitle,
  ipc,
}: {
  link: string;
  /** Names the sheet on iOS. The button's own name is its visible label. */
  sheetTitle: string;
  ipc: IpcSurface;
}): React.ReactElement | null {
  const { isIos } = usePlatform();
  const route = shareLinkRoute(isIos, ipc, globalThis.navigator);
  // Same transient-flag shape as useCopyFeedback: the button IS the feedback
  // surface, so a failure needs no new layout in the actions row.
  const [failed, setFailed] = React.useState(false);
  const failTimer = React.useRef<number | null>(null);
  React.useEffect(
    () => () => {
      if (failTimer.current) window.clearTimeout(failTimer.current);
    },
    [],
  );
  if (route === "unavailable") return null;

  const reportFailure = (err: unknown) => {
    // A dismissal never reaches here — shareLinkViaWebShare swallows AbortError.
    console.error("vlerv: share link failed", err);
    setFailed(true);
    if (failTimer.current) window.clearTimeout(failTimer.current);
    failTimer.current = window.setTimeout(() => setFailed(false), FAIL_RESET_MS);
  };

  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (route === "web-share") {
      void shareLinkViaWebShare(globalThis.navigator, link, sheetTitle).catch(reportFailure);
      return;
    }
    void ipc.shareLink?.(link, shareAnchorFrom(e.currentTarget)).catch(reportFailure);
  };

  return (
    <button
      type="button"
      className={"button button-secondary" + (failed ? " button-danger" : "")}
      data-testid="share-link"
      title="Share…"
      onClick={handleClick}
    >
      <Share size={13} strokeWidth={2} />
      {failed ? "Share failed" : "Share"}
    </button>
  );
}
