// Small pure formatting helpers shared by the views.

/** Format an epoch-millisecond timestamp as a compact local `YYYY-MM-DD HH:mm`, or `""` if absent. */
export function fmtDate(ms: number | null | undefined): string {
  if (ms == null) return "";
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
