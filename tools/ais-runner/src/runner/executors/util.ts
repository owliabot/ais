export function getString(obj: Record<string, unknown>, key: string): string | undefined {
  const value = obj[key];
  return typeof value === 'string' ? value : undefined;
}

export function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

export function isNumberArray(value: unknown): value is number[] {
  return Array.isArray(value) && value.every((entry) => Number.isFinite(entry));
}

export function expandTilde(path: string): string {
  if (!path.startsWith('~/')) return path;
  const env = typeof process !== 'undefined' ? process.env : undefined;
  const home = env?.HOME;
  if (!home) return path;
  return `${home}${path.slice(1)}`;
}
