const INITIAL_CHARACTER = /[\p{L}\p{N}]/u;

export function initials(name: string) {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => Array.from(part).find((character) => INITIAL_CHARACTER.test(character))?.toUpperCase())
    .filter(Boolean)
    .join("");
}

export function formatDate(value: string | null, options?: Intl.DateTimeFormatOptions) {
  if (!value) return "Never";
  const date = new Date(value.includes("T") ? value : value.replace(" ", "T") + "Z");
  if (Number.isNaN(date.valueOf())) return value;
  return new Intl.DateTimeFormat("en", options ?? { dateStyle: "medium" }).format(date);
}

export function relativeDays(days: number | null) {
  if (days === null) return "No interactions yet";
  const rounded = Math.max(0, Math.round(days));
  if (rounded === 0) return "Today";
  if (rounded === 1) return "Yesterday";
  return `${rounded} days ago`;
}

export function titleCase(value: string | null) {
  return value ? value.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase()) : "Unknown";
}
