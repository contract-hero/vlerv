import { describe, expect, it } from "vitest";
import { arrivalOpensTab, RECEIVE_BURST_MS } from "./beam-burst";

/** The provider's whole loop over a delivery run: fold arrival times through
 *  the same "remember this one, then judge the next" step BeamProvider does
 *  with its ref, and report which arrivals opened a tab. */
function arrivalsThatOpenTabs(arrivalTimes: number[]): number[] {
  let previous: number | null = null;
  const opened: number[] = [];
  for (const at of arrivalTimes) {
    if (arrivalOpensTab(previous, at)) opened.push(at);
    previous = at;
  }
  return opened;
}

describe("arrivalOpensTab — which pushed artifact opens a tab", () => {
  const t0 = 1_700_000_000_000;

  it("opens a tab for the first arrival, with nothing before it", () => {
    expect(arrivalOpensTab(null, t0)).toBe(true);
  });

  it("does not open a tab for an arrival inside the window", () => {
    expect(arrivalOpensTab(t0, t0 + 1)).toBe(false);
    expect(arrivalOpensTab(t0, t0 + RECEIVE_BURST_MS - 1)).toBe(false);
  });

  it("opens a tab for an arrival outside the window", () => {
    expect(arrivalOpensTab(t0, t0 + RECEIVE_BURST_MS)).toBe(true);
    expect(arrivalOpensTab(t0, t0 + 60_000)).toBe(true);
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
    expect(arrivalOpensTab(t0, t0 - 1)).toBe(false);
    expect(arrivalOpensTab(t0, t0 - 60 * 60_000)).toBe(false);
    const stepBack = [t0, t0 - 3_600_000];
    expect(arrivalsThatOpenTabs(stepBack)).toEqual([t0]);
  });
});
