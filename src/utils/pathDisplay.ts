export function pathBasename(path: string): string {
  return path.split(/[/\\]+/).filter(Boolean).pop() || path;
}

export function pathTail(path: string, count: number): string {
  const parts = path.split(/[/\\]+/).filter(Boolean);
  return parts.slice(-count).join("/") || path;
}

function normalizeComparablePath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
}

export function isPathWithinMount(path: string, mountPoint: string): boolean {
  const normalizedPath = normalizeComparablePath(path);
  const normalizedMount = normalizeComparablePath(mountPoint);
  if (!normalizedPath || !normalizedMount) return false;
  return (
    normalizedPath === normalizedMount ||
    normalizedPath.startsWith(`${normalizedMount}/`)
  );
}
