// Address contract for Scope v2 remote tabs (design §6): opening a peer's
// artifact creates a tab whose address is `vlerv-remote://<peer-id>/abs/path`.
// The whole render pipeline (loader, scroll memory, isolation) keys off this
// string exactly like a local path — these are the only two places that need
// to know the scheme exists.
export const REMOTE_SCHEME = "vlerv-remote://";

export interface RemoteAddress {
  peer: string;
  /** The path AS SEEN ON THE HOST — always absolute. */
  path: string;
}

/** `vlerv-remote://<peer-id>/abs/path` — the peer id is the authority, the
 * remote path (already absolute) is used as-is. */
export function formatRemoteAddress(peer: string, path: string): string {
  return `${REMOTE_SCHEME}${peer}${path}`;
}

export function isRemoteAddress(address: string): boolean {
  return address.startsWith(REMOTE_SCHEME);
}

/** Parses a `vlerv-remote://` address; `null` for anything else, including a
 * malformed one (empty peer id or a path that lost its leading slash). The
 * single `slash <= 0` test rules out both: a slash at index 0 means an empty
 * peer id, and no slash at all means there is no absolute path to take. */
export function parseRemoteAddress(address: string): RemoteAddress | null {
  if (!isRemoteAddress(address)) return null;
  const rest = address.slice(REMOTE_SCHEME.length);
  const slash = rest.indexOf("/");
  if (slash <= 0) return null;
  return { peer: rest.slice(0, slash), path: rest.slice(slash) };
}
