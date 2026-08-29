// Pure display helpers for Beam UI (dialog + offers indicator). Size
// *limits* live in the backend (crates/vlerv-remote/src/beam.rs) — the
// receive dialog gets a pre-computed `warn` flag instead of mirroring the
// threshold here.

/** "482 KB", "1.2 MB" — coarse on purpose, one decimal only when it matters. */
export function humanBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "?";
  if (n < 1024) return `${n} B`;
  const kib = n / 1024;
  if (kib < 1024) return `${Math.round(kib)} KB`;
  const mib = kib / 1024;
  if (mib < 10) return `${mib.toFixed(1)} MB`;
  if (mib < 1024) return `${Math.round(mib)} MB`;
  return `${(mib / 1024).toFixed(1)} GB`;
}

/** Time until an offer's expiry, coarse on purpose: "23 h", "45 min",
 * "<1 min", "expired". Both arguments are unix seconds. */
export function expiresIn(expiresAt: number, nowSecs: number): string {
  const left = expiresAt - nowSecs;
  if (left <= 0) return "expired";
  if (left < 60) return "<1 min";
  if (left < 3600) return `${Math.round(left / 60)} min`;
  return `${Math.round(left / 3600)} h`;
}
