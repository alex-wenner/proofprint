// Text formatting shared by the views. Pure functions; no DOM.

/** `f1c28809…66ae`. */
export function shortHash(hex: string): string {
  return hex.length <= 14 ? hex : `${hex.slice(0, 8)}…${hex.slice(-4)}`;
}

/** `ppr1:f1c28809…66ae`. Keeps the prefix so record, blob, and key ids stay distinguishable. */
export function shortId(id: string): string {
  const prefix = id.slice(0, id.indexOf(":") + 1);
  return prefix + shortHash(id.slice(prefix.length));
}

/** Binary units, one decimal below 10. */
export function bytes(size: number): string {
  if (size < 1024) return `${size} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

/** `2026-01-15T12:00:00Z` → `2026-01-15 12:00:00 UTC`. No locale conversion. */
export function timestamp(iso: string): string {
  return iso.replace("T", " ").replace(/Z$/, " UTC");
}

/** Cut to `max` characters, counting code points so emoji are not split. */
export function truncate(text: string, max: number): string {
  const characters = Array.from(text);
  return characters.length <= max ? text : `${characters.slice(0, max - 1).join("")}…`;
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

/** Escape a value for one URL path segment, leaving `:` readable. */
export function segment(value: string): string {
  return encodeURIComponent(value).replace(/%3A/gi, ":");
}

export function recordPath(id: string): string {
  return `/records/${segment(id)}`;
}
