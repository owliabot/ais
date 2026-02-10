export interface CELDecimal {
  kind: 'decimal';
  int: bigint;
  /** Number of fractional digits (>= 0) */
  scale: number;
}

export function isCELDecimal(v: unknown): v is CELDecimal {
  return (
    !!v &&
    typeof v === 'object' &&
    (v as { kind?: unknown }).kind === 'decimal' &&
    typeof (v as { int?: unknown }).int === 'bigint' &&
    typeof (v as { scale?: unknown }).scale === 'number'
  );
}

export function parseNumericLiteral(raw: string): bigint | CELDecimal {
  if (raw.includes('.')) {
    const d = parseDecimalString(raw);
    return d.scale === 0 ? d.int : d;
  }
  return BigInt(raw);
}

export function parseDecimalString(raw: string): CELDecimal {
  if (typeof raw !== 'string') throw new Error('decimal must be string');
  if (raw.length === 0) throw new Error('decimal string is empty');
  if (raw.toLowerCase().includes('e')) throw new Error('decimal exponent notation is not allowed');
  // Strict decimal string: optional leading '-', digits, optional fractional part with digits.
  // Disallows: ".5", "1.", "+1", whitespace, exponent notation.
  if (!/^-?\d+(\.\d+)?$/.test(raw)) throw new Error(`invalid decimal: ${raw}`);

  const sign = raw.startsWith('-') ? -1n : 1n;
  const body = raw.startsWith('-') ? raw.slice(1) : raw;
  const [wholeRaw, fracRaw = ''] = body.split('.');

  const whole = wholeRaw.length === 0 ? '0' : wholeRaw;
  const frac = fracRaw;
  if (!/^\d+$/.test(whole) || (frac.length > 0 && !/^\d+$/.test(frac))) {
    throw new Error(`invalid decimal: ${raw}`);
  }

  const digits = (whole + frac).replace(/^0+(?=\d)/, '');
  const int = digits.length === 0 ? 0n : BigInt(digits) * sign;
  const scale = frac.length;
  return normalizeDecimal({ kind: 'decimal', int, scale });
}

export function decimalToString(d: CELDecimal): string {
  const norm = normalizeDecimal(d);
  if (norm.scale === 0) return norm.int.toString();

  const sign = norm.int < 0n ? '-' : '';
  const abs = norm.int < 0n ? -norm.int : norm.int;
  const s = abs.toString();

  const scale = norm.scale;
  if (s.length <= scale) {
    const zeros = '0'.repeat(scale - s.length);
    return `${sign}0.${zeros}${s}`;
  }

  const whole = s.slice(0, s.length - scale);
  const frac = s.slice(s.length - scale);
  return `${sign}${whole}.${frac}`;
}

export function normalizeDecimal(d: CELDecimal): CELDecimal {
  if (d.scale < 0 || !Number.isInteger(d.scale)) {
    throw new Error('decimal.scale must be a non-negative integer');
  }
  if (d.int === 0n) return { kind: 'decimal', int: 0n, scale: 0 };

  let int = d.int;
  let scale = d.scale;
  while (scale > 0 && int % 10n === 0n) {
    int /= 10n;
    scale -= 1;
  }
  return { kind: 'decimal', int, scale };
}

export function toDecimal(v: bigint | CELDecimal): CELDecimal {
  if (typeof v === 'bigint') return { kind: 'decimal', int: v, scale: 0 };
  return v;
}

export function decimalAdd(a: bigint | CELDecimal, b: bigint | CELDecimal): CELDecimal {
  const da = toDecimal(a);
  const db = toDecimal(b);
  const scale = Math.max(da.scale, db.scale);
  const ai = da.int * pow10(scale - da.scale);
  const bi = db.int * pow10(scale - db.scale);
  return normalizeDecimal({ kind: 'decimal', int: ai + bi, scale });
}

export function decimalSub(a: bigint | CELDecimal, b: bigint | CELDecimal): CELDecimal {
  const da = toDecimal(a);
  const db = toDecimal(b);
  const scale = Math.max(da.scale, db.scale);
  const ai = da.int * pow10(scale - da.scale);
  const bi = db.int * pow10(scale - db.scale);
  return normalizeDecimal({ kind: 'decimal', int: ai - bi, scale });
}

export function decimalMul(a: bigint | CELDecimal, b: bigint | CELDecimal): CELDecimal {
  const da = toDecimal(a);
  const db = toDecimal(b);
  return normalizeDecimal({
    kind: 'decimal',
    int: da.int * db.int,
    scale: da.scale + db.scale,
  });
}

export function decimalDiv(a: bigint | CELDecimal, b: bigint | CELDecimal): CELDecimal {
  const da = toDecimal(a);
  const db = toDecimal(b);
  if (db.int === 0n) throw new Error('Division by zero');

  // (da.int / 10^da.scale) / (db.int / 10^db.scale) = (da.int * 10^db.scale) / (db.int * 10^da.scale)
  let num = da.int * pow10(db.scale);
  let den = db.int * pow10(da.scale);

  // Normalize signs
  if (den < 0n) {
    den = -den;
    num = -num;
  }

  const g = gcd(absBigInt(num), den);
  num /= g;
  den /= g;

  // Only denominators with prime factors 2 and 5 can be represented as finite decimals.
  let den2 = den;
  const twos = factorCount(den2, 2n);
  den2 /= 2n ** BigInt(twos);
  const fives = factorCount(den2, 5n);
  den2 /= 5n ** BigInt(fives);
  if (den2 !== 1n) {
    throw new Error('Non-terminating decimal division; use mul_div() for integer math');
  }

  const scale = Math.max(twos, fives);
  const adj2 = scale - twos;
  const adj5 = scale - fives;

  const scaledNum = num * (2n ** BigInt(adj2)) * (5n ** BigInt(adj5));
  return normalizeDecimal({ kind: 'decimal', int: scaledNum, scale });
}

export function decimalCompare(a: bigint | CELDecimal, b: bigint | CELDecimal): -1 | 0 | 1 {
  const da = toDecimal(a);
  const db = toDecimal(b);
  const scale = Math.max(da.scale, db.scale);
  const ai = da.int * pow10(scale - da.scale);
  const bi = db.int * pow10(scale - db.scale);
  if (ai < bi) return -1;
  if (ai > bi) return 1;
  return 0;
}

export function decimalAbs(a: bigint | CELDecimal): CELDecimal {
  const da = toDecimal(a);
  return da.int < 0n ? { kind: 'decimal', int: -da.int, scale: da.scale } : da;
}

export function decimalFloor(d: CELDecimal): bigint {
  const norm = normalizeDecimal(d);
  if (norm.scale === 0) return norm.int;
  const div = pow10(norm.scale);
  const q = norm.int / div; // trunc toward zero
  const r = norm.int % div;
  if (r === 0n) return q;
  // If negative and has remainder, floor is one less than trunc
  return norm.int < 0n ? q - 1n : q;
}

export function decimalCeil(d: CELDecimal): bigint {
  const norm = normalizeDecimal(d);
  if (norm.scale === 0) return norm.int;
  const div = pow10(norm.scale);
  const q = norm.int / div;
  const r = norm.int % div;
  if (r === 0n) return q;
  // If positive and has remainder, ceil is one more than trunc
  return norm.int > 0n ? q + 1n : q;
}

export function decimalRound(d: CELDecimal): bigint {
  const norm = normalizeDecimal(d);
  if (norm.scale === 0) return norm.int;
  const div = pow10(norm.scale);
  const q = norm.int / div; // trunc toward zero
  const r = absBigInt(norm.int % div);
  const half = div / 2n;
  if (r < half) return q;
  if (r > half) return norm.int >= 0n ? q + 1n : q - 1n;
  // exactly half: round away from zero
  return norm.int >= 0n ? q + 1n : q - 1n;
}

export function pow10(n: number): bigint {
  if (!Number.isInteger(n) || n < 0) throw new Error('pow10 requires non-negative integer');
  const cached = POW10_CACHE.get(n);
  if (cached !== undefined) return cached;
  let v = 1n;
  for (let i = 0; i < n; i++) v *= 10n;
  POW10_CACHE.set(n, v);
  return v;
}

const POW10_CACHE = new Map<number, bigint>([[0, 1n]]);

function absBigInt(n: bigint): bigint {
  return n < 0n ? -n : n;
}

function gcd(a: bigint, b: bigint): bigint {
  let x = a;
  let y = b;
  while (y !== 0n) {
    const t = x % y;
    x = y;
    y = t;
  }
  return x;
}

function factorCount(n: bigint, p: bigint): number {
  let x = n;
  let count = 0;
  while (x !== 0n && x % p === 0n) {
    x /= p;
    count++;
  }
  return count;
}
