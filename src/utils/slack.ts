// Slack deep-link helpers for the share feature. Slack ships no macOS share
// extension, so the native share sheet can't reach it — the bridge is a
// slack://channel deep link that foregrounds Slack on a configured channel,
// after which the user drags the file in (drag-out already works from the
// explorer tree).

/**
 * Expand the user-configured Slack target preference into an openable
 * slack:// URL. Accepted forms:
 *  - a full `slack://…` URL (passed through untouched)
 *  - `TEAMID/CHANNELID` shorthand (e.g. `T0123ABCD/C0456EFGH`) — expands to
 *    `slack://channel?team=…&id=…`. A `D…` id (DM) expands via slack://user.
 * Anything else returns null and the affordance stays hidden.
 */
export function slackUrlFromTarget(target: string): string | null {
  const trimmed = target.trim();
  if (trimmed.length === 0) return null;
  if (trimmed.startsWith("slack://")) return trimmed;

  const match = /^(T[A-Z0-9]+)\/([CDG][A-Z0-9]+)$/i.exec(trimmed);
  if (!match) return null;
  const team = match[1].toUpperCase();
  const id = match[2].toUpperCase();
  if (id.startsWith("D")) {
    return `slack://user?team=${team}&id=${id}`;
  }
  return `slack://channel?team=${team}&id=${id}`;
}
