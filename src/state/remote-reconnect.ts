// Pure reconnect policy for RemoteProvider — design §4's lazy-boot rule,
// mirrored client-side. Kept dependency-free for the same reason
// remote-tabs.ts is: the one decision that can leave every peer drawer empty
// for the rest of a session is testable here without a provider, an IPC mock,
// or an event listener.

/** Why a reconnect is running. */
export type ReconnectReason =
  /** The provider mounted: this app has just started and holds no sessions. */
  | "launch"
  /**
   * iOS handed the app back to the foreground. The backend has already
   * dropped every session it held, and the provider has just cleared its
   * subscription set to match.
   */
  | "resume";

/**
 * Does this reconnect dial every paired peer?
 *
 * At LAUNCH the answer is the user's `remote_listen` preference. That is the
 * lazy-boot rule: with listening off, nothing dials until the user asks for
 * it — "Connect" in Settings, or opening anything that calls `subscribePeer`.
 *
 * On a RESUME the answer is always yes and the preference is not read. The
 * sessions are KNOWN to be gone and the subscription set has just been
 * cleared, so a preference gate here re-establishes NOTHING: every drawer
 * stays empty until the user quits and launches the app again, and the only
 * platform that resumes is the only one that suffers it. The preference
 * guards a first socket at launch, and an app that already held live sessions
 * is long past that.
 */
export function dialsEveryPeer(reason: ReconnectReason, listenAtLaunch: boolean): boolean {
  return reason === "resume" || listenAtLaunch;
}
