export function parseJsonObject(
  raw: string,
  flagName: string,
  parse: (json: string) => unknown = JSON.parse
): Record<string, unknown> {
  let parsed: unknown;
  try {
    parsed = parse(raw);
  } catch (error) {
    throw new Error(`${flagName} must be valid JSON: ${(error as Error).message}`);
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error(`${flagName} must be a JSON object`);
  }
  return parsed as Record<string, unknown>;
}
