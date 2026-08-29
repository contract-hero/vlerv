import { describe, expect, it } from "vitest";
import { formatRemoteAddress, isRemoteAddress, parseRemoteAddress } from "./remote-address";

describe("formatRemoteAddress / parseRemoteAddress", () => {
  it("round-trips a peer id and an absolute path", () => {
    const address = formatRemoteAddress("abcd1234", "/w/project/report.html");
    expect(address).toBe("vlerv-remote://abcd1234/w/project/report.html");
    expect(parseRemoteAddress(address)).toEqual({
      peer: "abcd1234",
      path: "/w/project/report.html",
    });
  });

  it("round-trips a root-level path", () => {
    const address = formatRemoteAddress("peer", "/a.md");
    expect(parseRemoteAddress(address)).toEqual({ peer: "peer", path: "/a.md" });
  });

  it("recognizes the scheme", () => {
    expect(isRemoteAddress("vlerv-remote://peer/a.md")).toBe(true);
    expect(isRemoteAddress("/local/a.md")).toBe(false);
    expect(isRemoteAddress("file:///local/a.md")).toBe(false);
  });

  it("rejects non-remote addresses", () => {
    expect(parseRemoteAddress("/local/a.md")).toBeNull();
    expect(parseRemoteAddress("vlerv://open?path=/a")).toBeNull();
  });

  it("rejects a malformed remote address (no peer, or a path missing its slash)", () => {
    expect(parseRemoteAddress("vlerv-remote:///a.md")).toBeNull();
    expect(parseRemoteAddress("vlerv-remote://peer")).toBeNull();
    expect(parseRemoteAddress("vlerv-remote://peer/")).toEqual({ peer: "peer", path: "/" });
  });
});
