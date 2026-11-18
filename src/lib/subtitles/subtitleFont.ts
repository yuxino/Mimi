/** Keep one installed family name, never a caller-supplied CSS font stack. */
export function normalizeSubtitleFontFamily(value?: string): string {
  const family = value?.trim() ?? "";
  return Array.from(family).length <= 256 && !/\p{Cc}/u.test(value ?? "") ? family : "";
}

/** Quote font names so commas, quotes and CSS keywords remain literal names.
 * The language-aware UI stack supplies missing glyphs and removed fonts. */
export function subtitleFontFamily(value?: string): string {
  const family = normalizeSubtitleFontFamily(value);
  if (!family) return "var(--mimi-ui-font)";
  return `"${family.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}", var(--mimi-ui-font)`;
}
