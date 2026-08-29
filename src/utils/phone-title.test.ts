import { describe, expect, it } from "vitest";
import { phoneTitle } from "./phone-title";

describe("phoneTitle", () => {
  it("is null for the start page (no address)", () => {
    expect(phoneTitle(null)).toBeNull();
  });
  it("shows the basename of a local path", () => {
    expect(phoneTitle("/Users/v/reports/audit.html")).toBe("audit.html");
  });
  it("resolves a remote address to the host-side basename", () => {
    expect(phoneTitle("vlerv-remote://abc123/home/v/report.html")).toBe("report.html");
  });
  it("falls back to the basename of the raw address when the remote form is malformed", () => {
    expect(phoneTitle("vlerv-remote://nopath")).toBe("nopath");
  });
});
