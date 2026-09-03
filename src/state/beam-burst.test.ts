import { describe, expect, it } from "vitest";
import { nextBurst, RECEIVE_BURST_MS } from "./beam-burst";

/** A delivery run, driven the way BeamProvider drives it: hold `previous`,
 *  hand it to `nextBurst`, keep what comes back. The fold under test is
 *  inside `nextBurst` — this only carries its answer forward. */
function arrivalsThatOpenTabs(arrivalTimes: number[]): number[] {
  let previous: number | null = null;
  const opened: number[] = [];
  for (const at of arrivalTimes) {
    const burst = nextBurst(previous, at);
    if (burst.opens) opened.push(at);
    previous = burst.previous;
  }
  return opened;
}

describe("nextBurst — which pushed artifact opens a tab", () => {
  const t0 = 1_700_000_000_000;

  it("opens a tab for the first arrival, with nothing before it", () => {
    expect(nextBurst(null, t0).opens).toBe(true);
  });

  it("does not open a tab for an arrival inside the window", () => {
    expect(nextBurst(t0, t0 + 1).opens).toBe(false);
    expect(nextBurst(t0, t0 + RECEIVE_BURST_MS - 1).opens).toBe(false);
  });

  it("opens a tab for an arrival outside the window", () => {
    expect(nextBurst(t0, t0 + RECEIVE_BURST_MS).opens).toBe(true);
    expect(nextBurst(t0, t0 + 60_000).opens).toBe(true);
  });

  it("remembers an arrival that opened no tab, so the next one is measured from it", () => {
    // The half of the rule a caller cannot see it is missing. Remember only
    // the arrivals that opened a tab and every case above still passes, while
    // a drain opens one more tab every RECEIVE_BURST_MS for as long as it
    // runs.
    const inRun = nextBurst(t0, t0 + 1);
    expect(inRun.opens).toBe(false);
    expect(inRun.previous).toBe(t0 + 1);
    expect(nextBurst(null, t0).previous).toBe(t0);
  });

  it("keeps one run of arrivals to one tab even when the run outlasts the window", () => {
    // Eight queued files draining one every 3 s: 21 s of arrivals, each one
    // inside the window of the previous one. The window is measured from the
    // previous arrival for exactly this case — measured from the first, the
    // drain would start opening tabs partway through and bury the user under
    // the stack this rule exists to prevent.
    const drain = [0, 3_000, 6_000, 9_000, 12_000, 15_000, 18_000, 21_000].map((d) => t0 + d);
    expect(arrivalsThatOpenTabs(drain)).toEqual([t0]);
  });

  it("opens a second tab once a run stops and a later push arrives on its own", () => {
    const drain = [t0, t0 + 3_000, t0 + 6_000];
    const later = t0 + 6_000 + RECEIVE_BURST_MS;
    expect(arrivalsThatOpenTabs([...drain, later])).toEqual([t0, later]);
  });

  it("does not open a second tab when the clock steps backwards mid-run", () => {
    // A clock correction between two arrivals of one run gives a negative
    // gap. Read as an absolute distance it would look like a fresh push and
    // open a tab the sender never asked for, so a backwards step stays inside
    // the window.
    expect(nextBurst(t0, t0 - 1).opens).toBe(false);
    expect(nextBurst(t0, t0 - 60 * 60_000).opens).toBe(false);
    const stepBack = [t0, t0 - 3_600_000];
    expect(arrivalsThatOpenTabs(stepBack)).toEqual([t0]);
  });
});
