/** Release notes are Markdown; the update window shows them as plain text. */

const CR = /\r/g;
const HEADING = /^#{1,6}[ \t]*/gm;
const BULLET = /^[ \t]*[-*][ \t]+/gm;
const BOLD = /\*\*(.+?)\*\*/g;
const LINK = /\[([^\]]+)\]\([^)]+\)/g;
const CODE = /`([^`]+)`/g;
const GAPS = /\n{3,}/g;

/** Markdown to plain text: no # or ** marks, links shown by their text, "•" bullets. */
export function plainNotes(md: string): string {
  return md
    .replace(CR, "")
    .replace(HEADING, "")
    .replace(BULLET, "• ")
    .replace(BOLD, "$1")
    .replace(LINK, "$1")
    .replace(CODE, "$1")
    .replace(GAPS, "\n\n")
    .trim();
}
