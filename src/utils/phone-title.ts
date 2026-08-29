// Title-band label for the phone shell: the artifact's basename, with
// `vlerv-remote://` addresses resolved to the path as seen on the host.
import { basename } from "./path";
import { parseRemoteAddress } from "./remote-address";

export function phoneTitle(address: string | null): string | null {
  if (!address) return null;
  const remote = parseRemoteAddress(address);
  return basename(remote ? remote.path : address);
}
