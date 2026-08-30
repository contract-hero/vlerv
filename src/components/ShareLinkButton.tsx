// ShareLinkButton — the companion to CopyLinkButton. Copy puts the link on
// THIS machine's clipboard, but a pairing link's whole job is to reach the
// OTHER device: the share sheet does that in one step (AirDrop, Messages,
// Mail) instead of a clipboard plus a second app.
//
// Rendering nothing when no route exists is deliberate — a share button that
// silently does nothing on tap is worse than no button (see `shareLinkRoute`).
import * as React from "react";
import { Share } from "lucide-react";
import { tauriIpc } from "../ipc";
import type { IpcSurface } from "../ipc";
import { usePlatform } from "../state/platform";
import { shareLinkRoute, shareLinkViaWebShare } from "../utils/share-link";

export default function ShareLinkButton({
  link,
  title,
  ipc = tauriIpc,
}: {
  link: string;
  /** Names the sheet on iOS, and the button for a screen reader. */
  title: string;
  ipc?: IpcSurface;
}): React.ReactElement | null {
  const { isIos } = usePlatform();
  const route = shareLinkRoute(isIos, ipc, globalThis.navigator);
  if (route === "unavailable") return null;

  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (route === "web-share") {
      // Share failures stay silent, matching Preview's Share button: the OS
      // sheet already reported anything the user needs to see.
      void shareLinkViaWebShare(globalThis.navigator, link, title).catch(() => {});
      return;
    }
    // The anchor is viewport-relative and goes stale if the panel scrolls
    // between click and native display, so read it synchronously.
    const r = e.currentTarget.getBoundingClientRect();
    void ipc
      .shareLink?.(link, { x: r.x, y: r.y, width: r.width, height: r.height })
      .catch(() => {});
  };

  return (
    <button
      type="button"
      className="button button-secondary"
      data-testid="share-link"
      title="Share…"
      aria-label={title}
      onClick={handleClick}
    >
      <Share size={13} strokeWidth={2} />
      Share
    </button>
  );
}
