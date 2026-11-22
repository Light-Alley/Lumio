/** Path helpers that work for both "/" (macOS, Linux) and "\\" (Windows) paths. */

const TRAILING_SEPARATORS = /[\\/]+$/;

/** `path` is `folder` itself or somewhere inside it ("/Trip 2" is not inside "/Trip"). */
export function isInside(path: string, folder: string): boolean {
  if (path === folder) return true;
  const f = folder.replace(TRAILING_SEPARATORS, "");
  return path.startsWith(f + "/") || path.startsWith(f + "\\");
}
