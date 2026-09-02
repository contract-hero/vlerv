// Pure burst policy for the Beam receive path: which arrival of a delivery
// run opens a tab. Kept dependency-free for the same reason
// remote-reconnect.ts is — the one decision that can bury whatever the user
// is reading under a stack of tabs is testable here without a provider, an
// IPC mock, or an event listener.

/** How close two `vlerv://beam-received` arrivals must be to count as one
 *  delivery run rather than two things the sender asked for separately. Only
 *  the first artifact of a run opens a tab: too short and a slow run opens a
 *  second one, too long and a file pushed on its own lands with no tab and
 *  only a row in the Received list. */
export const RECEIVE_BURST_MS = 5000;

/**
 * Does this arrival open a tab?
 *
 * `previousArrivalAt` is when the previous artifact landed, in `Date.now()`
 * milliseconds, or null when nothing has landed yet — the first artifact
 * opens exactly like an accepted receive does.
 *
 * The window is measured from the previous arrival, not from the first of the
 * run, so a sender that queued eight files while this device was unreachable
 * still drains them as one burst even when the drain takes a minute.
 *
 * A clock that steps backwards between two arrivals gives a negative gap, and
 * that gap stays inside the window on purpose: a backwards jump is no
 * evidence the sender asked for a second file, and counting it as one opens
 * the very tab this rule exists to prevent.
 */
export function arrivalOpensTab(previousArrivalAt: number | null, arrivalAt: number): boolean {
  if (previousArrivalAt === null) return true;
  return arrivalAt - previousArrivalAt >= RECEIVE_BURST_MS;
}
