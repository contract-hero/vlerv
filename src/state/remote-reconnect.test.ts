import { describe, expect, it } from "vitest";
import { dialsEveryPeer } from "./remote-reconnect";

describe("dialsEveryPeer — who a reconnect dials", () => {
  it("dials every peer at launch when listening at launch is on", () => {
    expect(dialsEveryPeer("launch", true)).toBe(true);
  });

  it("dials nobody at launch when listening at launch is off", () => {
    // The lazy-boot rule, mirrored client-side: with the switch off, the
    // first socket waits for an explicit Connect.
    expect(dialsEveryPeer("launch", false)).toBe(false);
  });

  it("dials every peer on a resume even when listening at launch is off", () => {
    // The resume path has just cleared the subscription set, because the
    // backend dropped every session it held. Reading the launch preference
    // here re-establishes NOTHING, and the drawers stay empty for the rest of
    // the session — on the one platform that resumes at all.
    expect(dialsEveryPeer("resume", false)).toBe(true);
  });

  it("dials every peer on a resume when listening at launch is on", () => {
    expect(dialsEveryPeer("resume", true)).toBe(true);
  });
});
