// The kinds of rule a smart collection can use. Each entry here matches arms of
// `rule_sql` in src-tauri/src/collections.rs; to add a field, add both.
import { LABELS, type Rule, type RuleSet } from "./api";

/** How the value is typed: a number, text, a date, a choice, or nothing. */
export type ValueKind = "number" | "text" | "date" | "choice" | "none" | "folder" | "stars";
export interface Op { op: string; label: string; value: ValueKind; between?: boolean; choices?: [string, string][]; placeholder?: string }
export interface Field { field: string; label: string; ops: Op[] }

const MONTHS: [string, string][] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
  .map((m, i) => [String(i + 1), m]);
const LABEL_CHOICES: [string, string][] = LABELS.map((l) => [l.name, l.name]);
const TYPES: [string, string][] = [
  ["photo", "Photo (not video)"], ["raw", "RAW"], ["jpeg", "JPEG"], ["heic", "HEIC / AVIF"], ["png", "PNG"], ["tiff", "TIFF"], ["video", "Video"],
];
const minMax = (unit: string): Op[] => [
  { op: "gte", label: "at least", value: "number", placeholder: unit },
  { op: "lte", label: "at most", value: "number", placeholder: unit },
];

export const FIELDS: Field[] = [
  { field: "rating", label: "Rating", ops: [
    { op: "gte", label: "at least", value: "stars" },
    { op: "eq", label: "exactly", value: "stars" },
    { op: "lte", label: "at most", value: "stars" },
  ] },
  { field: "taken", label: "Date taken", ops: [
    { op: "year", label: "in the year", value: "number", placeholder: "2024" },
    { op: "month", label: "in the month of", value: "choice", choices: MONTHS },
    { op: "between", label: "between", value: "date", between: true },
    { op: "after", label: "on or after", value: "date" },
    { op: "before", label: "on or before", value: "date" },
    { op: "last_days", label: "in the last … days", value: "number", placeholder: "30" },
    { op: "none", label: "is unknown", value: "none" },
  ] },
  { field: "label", label: "Colour label", ops: [
    { op: "is", label: "is", value: "choice", choices: LABEL_CHOICES },
    { op: "is_not", label: "is not", value: "choice", choices: LABEL_CHOICES },
    { op: "any", label: "has any label", value: "none" },
    { op: "none", label: "has no label", value: "none" },
  ] },
  { field: "keyword", label: "Keyword", ops: [
    { op: "has", label: "includes", value: "text", placeholder: "Italy" },
    { op: "has_not", label: "doesn't include", value: "text", placeholder: "Italy" },
    { op: "any", label: "has any keyword", value: "none" },
    { op: "none", label: "has no keywords", value: "none" },
  ] },
  { field: "folder", label: "Folder", ops: [
    { op: "in", label: "is in (with subfolders)", value: "folder" },
    { op: "not_in", label: "is not in", value: "folder" },
  ] },
  { field: "camera", label: "Camera", ops: [
    { op: "contains", label: "contains", value: "text", placeholder: "EOS R5" },
    { op: "not_contains", label: "doesn't contain", value: "text", placeholder: "iPhone" },
    { op: "none", label: "is unknown", value: "none" },
  ] },
  { field: "lens", label: "Lens", ops: [
    { op: "contains", label: "contains", value: "text", placeholder: "70-200" },
    { op: "none", label: "is unknown", value: "none" },
  ] },
  { field: "type", label: "File type", ops: [
    { op: "is", label: "is", value: "choice", choices: TYPES },
    { op: "is_not", label: "is not", value: "choice", choices: TYPES },
  ] },
  { field: "text", label: "Any text", ops: [
    { op: "contains", label: "contains", value: "text", placeholder: "name, title, caption, keyword or place" },
    { op: "not_contains", label: "doesn't contain", value: "text" },
  ] },
  { field: "filename", label: "File name", ops: [
    { op: "contains", label: "contains", value: "text", placeholder: "IMG_" },
    { op: "starts", label: "starts with", value: "text", placeholder: "DSC" },
    { op: "not_contains", label: "doesn't contain", value: "text" },
  ] },
  { field: "location", label: "Location", ops: [
    { op: "yes", label: "is on the map", value: "none" },
    { op: "no", label: "is missing", value: "none" },
  ] },
  { field: "place", label: "Place name", ops: [{ op: "contains", label: "contains", value: "text", placeholder: "London" }] },
  { field: "shape", label: "Shape", ops: [
    { op: "landscape", label: "is landscape", value: "none" },
    { op: "portrait", label: "is portrait", value: "none" },
    { op: "square", label: "is square", value: "none" },
  ] },
  { field: "added", label: "Added to Lumio", ops: [{ op: "last_days", label: "in the last … days", value: "number", placeholder: "30" }] },
  { field: "iso", label: "ISO", ops: minMax("800") },
  { field: "focal", label: "Focal length (mm)", ops: minMax("50") },
  { field: "fnumber", label: "Aperture (f/)", ops: minMax("2.8") },
];

export const fieldOf = (r: Rule) => FIELDS.find((f) => f.field === r.field) ?? FIELDS[0];
export const opOf = (r: Rule) => fieldOf(r).ops.find((o) => o.op === r.op) ?? fieldOf(r).ops[0];

/** A new rule for a field, with its first condition and a sensible starting value. */
export function newRule(field = "rating"): Rule {
  const f = FIELDS.find((x) => x.field === field) ?? FIELDS[0];
  const op = f.ops[0];
  return { field: f.field, op: op.op, value: startValue(op), value2: "" };
}

export function startValue(op: Op): string {
  if (op.value === "stars") return "5";
  if (op.value === "choice") return op.choices?.[0]?.[0] ?? "";
  if (op.op === "year") return String(new Date().getFullYear());
  return "";
}

/** Ready-made collections to start from. */
export const TEMPLATES: { name: string; rules: RuleSet }[] = [
  { name: `5 stars from ${new Date().getFullYear()}`, rules: { any: false, rules: [
    { field: "rating", op: "gte", value: "5", value2: "" },
    { field: "taken", op: "year", value: String(new Date().getFullYear()), value2: "" },
  ] } },
  { name: "Not rated yet", rules: { any: false, rules: [{ field: "rating", op: "eq", value: "0", value2: "" }] } },
  { name: "No keywords", rules: { any: false, rules: [{ field: "keyword", op: "none", value: "", value2: "" }] } },
  { name: "Last 30 days", rules: { any: false, rules: [{ field: "taken", op: "last_days", value: "30", value2: "" }] } },
  { name: "RAW files", rules: { any: false, rules: [{ field: "type", op: "is", value: "raw", value2: "" }] } },
  { name: "Videos", rules: { any: false, rules: [{ field: "type", op: "is", value: "video", value2: "" }] } },
];
