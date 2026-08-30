// ShareLinkButton — the companion to CopyLinkButton. Copy puts the link on
// THIS machine's clipboard, but a pairing link's whole job is to reach the
// OTHER device: the share sheet does that in one step (AirDrop, Messages,
// Mail) instead of a clipboard plus a second app.
//
// Rendering nothing when no route exists is deliberate — a share button that
// silently does nothing on tap is worse than no button (see `shareLinkRoute`).
import * as React from "react";
import { Share } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { usePlatform } from "../state/platform";
import { shareAnchorFrom, shareLinkRoute, shareLinkViaWebShare } from "../utils/share-link";

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
  if (route === "unavailable") return null;

  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (route === "web-share") {
      // Share failures stay silent, matching Preview's Share button: the OS
      // sheet already reported anything the user needs to see.
      void shareLinkViaWebShare(globalThis.navigator, link, sheetTitle).catch(() => {});
      return;
    }
    void ipc.shareLink?.(link, shareAnchorFrom(e.currentTarget)).catch(() => {});
  };

  return (
    <button
      type="button"
      className="button button-secondary"
      data-testid="share-link"
      title="Share…"
      onClick={handleClick}
    >
      <Share size={13} strokeWidth={2} />
      Share
    </button>
  );
}
