export type ValidationIssue = {
  path: Array<string | number>;
  message: string;
};

export const CAIP2_RE = /^[a-z0-9]+:.+$/i;

export function asNonEmptyString(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function asPositiveInt(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isInteger(value) || value <= 0) return null;
  return value;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

export function formatPath(path: Array<string | number>): string {
  if (!path.length) return '(root)';
  return path
    .map((part) =>
      typeof part === 'number'
        ? `[${part}]`
        : /^[A-Za-z_][A-Za-z0-9_]*$/.test(part)
          ? part
          : JSON.stringify(part)
    )
    .join('.');
}
