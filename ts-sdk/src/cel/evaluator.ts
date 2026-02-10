/**
 * CEL Evaluator - evaluate AST against a context
 */

import { Parser, type ASTNode } from './parser.js';
import type { CELDecimal } from './numeric.js';
import {
  isCELDecimal,
  parseDecimalString,
  decimalToString,
  normalizeDecimal,
  decimalAdd,
  decimalSub,
  decimalMul,
  decimalDiv,
  decimalCompare,
  decimalAbs,
  decimalFloor,
  decimalCeil,
  decimalRound,
  pow10,
} from './numeric.js';

export type CELValue =
  | string
  | bigint
  | CELDecimal
  | boolean
  | null
  | CELValue[]
  | { [key: string]: CELValue };

export type CELContext = Record<string, CELValue>;

type CELFunction = (args: CELValue[], ctx: CELContext) => CELValue;

const BUILTINS: Record<string, CELFunction> = {
  size: (args) => {
    const val = args[0];
    if (typeof val === 'string') return BigInt(val.length);
    if (Array.isArray(val)) return BigInt(val.length);
    if (val && typeof val === 'object') return BigInt(Object.keys(val).length);
    throw new Error('size() requires string, list, or map');
  },

  contains: (args) => {
    const [str, substr] = args;
    if (typeof str !== 'string' || typeof substr !== 'string') {
      throw new Error('contains() requires two strings');
    }
    return str.includes(substr);
  },

  startsWith: (args) => {
    const [str, prefix] = args;
    if (typeof str !== 'string' || typeof prefix !== 'string') {
      throw new Error('startsWith() requires two strings');
    }
    return str.startsWith(prefix);
  },

  endsWith: (args) => {
    const [str, suffix] = args;
    if (typeof str !== 'string' || typeof suffix !== 'string') {
      throw new Error('endsWith() requires two strings');
    }
    return str.endsWith(suffix);
  },

  matches: (args) => {
    const [str, pattern] = args;
    if (typeof str !== 'string' || typeof pattern !== 'string') {
      throw new Error('matches() requires two strings');
    }
    return new RegExp(pattern).test(str);
  },

  lower: (args) => {
    const str = args[0];
    if (typeof str !== 'string') throw new Error('lower() requires string');
    return str.toLowerCase();
  },

  upper: (args) => {
    const str = args[0];
    if (typeof str !== 'string') throw new Error('upper() requires string');
    return str.toUpperCase();
  },

  trim: (args) => {
    const str = args[0];
    if (typeof str !== 'string') throw new Error('trim() requires string');
    return str.trim();
  },

  int: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val;
    if (isCELDecimal(val)) return truncDecimalToInt(val);
    if (typeof val === 'string') {
      if (val.toLowerCase().includes('e')) throw new Error('int() does not allow exponent notation');
      if (/^-?\d+$/.test(val)) return BigInt(val);
      return truncDecimalToInt(parseDecimalString(val));
    }
    throw new Error('int() requires int/decimal/string');
  },

  uint: (args) => {
    const out = BUILTINS.int(args, {} as any);
    if (typeof out !== 'bigint') throw new Error('uint() internal error');
    return out < 0n ? -out : out;
  },

  double: (args) => {
    const val = args[0];
    if (isCELDecimal(val)) return val;
    if (typeof val === 'bigint') return { kind: 'decimal', int: val, scale: 0 };
    if (typeof val === 'string') return parseDecimalString(val);
    throw new Error('double() requires int/decimal/string');
  },

  string: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val.toString();
    if (isCELDecimal(val)) return decimalToString(val);
    return String(val);
  },

  bool: (args) => {
    const val = args[0];
    if (typeof val === 'boolean') return val;
    if (typeof val === 'string') return val === 'true';
    if (typeof val === 'bigint') return val !== 0n;
    if (isCELDecimal(val)) return normalizeDecimal(val).int !== 0n;
    return Boolean(val);
  },

  type: (args) => {
    const val = args[0];
    if (val === null) return 'null';
    if (Array.isArray(val)) return 'list';
    if (typeof val === 'bigint') return 'int';
    if (isCELDecimal(val)) return 'decimal';
    if (val && typeof val === 'object') return 'map';
    if (typeof val === 'boolean') return 'bool';
    return typeof val;
  },

  exists: (args) => {
    const [list] = args;
    if (!Array.isArray(list)) throw new Error('exists() requires list');
    return list.some((item) => Boolean(item));
  },

  all: (args) => {
    const [list] = args;
    if (!Array.isArray(list)) throw new Error('all() requires list');
    return list.every((item) => Boolean(item));
  },

  abs: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val < 0n ? -val : val;
    if (isCELDecimal(val)) return decimalAbs(val);
    throw new Error('abs() requires int or decimal');
  },

  min: (args) => {
    const nums = args.filter((a): a is bigint | CELDecimal => isNumeric(a));
    if (nums.length === 0) throw new Error('min() requires numeric args');
    let best = nums[0]!;
    for (const v of nums.slice(1)) {
      if (numericCompare(v, best) < 0) best = v;
    }
    return normalizeNumeric(best);
  },

  max: (args) => {
    const nums = args.filter((a): a is bigint | CELDecimal => isNumeric(a));
    if (nums.length === 0) throw new Error('max() requires numeric args');
    let best = nums[0]!;
    for (const v of nums.slice(1)) {
      if (numericCompare(v, best) > 0) best = v;
    }
    return normalizeNumeric(best);
  },

  floor: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val;
    if (isCELDecimal(val)) return decimalFloor(val);
    throw new Error('floor() requires int or decimal');
  },

  ceil: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val;
    if (isCELDecimal(val)) return decimalCeil(val);
    throw new Error('ceil() requires int or decimal');
  },

  round: (args) => {
    const val = args[0];
    if (typeof val === 'bigint') return val;
    if (isCELDecimal(val)) return decimalRound(val);
    throw new Error('round() requires int or decimal');
  },

  to_atomic: (args) => {
    const [amount, asset] = args;
    const decimals = getDecimals(asset);
    if (decimals < 0 || decimals > 77) throw new Error('to_atomic() decimals out of range');

    const dec = coerceAmountDecimal(amount);
    if (dec.int < 0n) throw new Error('to_atomic() amount must be non-negative');
    if (dec.scale > decimals) {
      throw new Error('to_atomic() disallows truncation: too many fractional digits');
    }
    return dec.int * pow10(decimals - dec.scale);
  },

  to_human: (args) => {
    const [atomic, asset] = args;
    const decimals = getDecimals(asset);
    if (decimals < 0 || decimals > 77) throw new Error('to_human() decimals out of range');
    const bi = coerceBigInt(atomic, 'to_human.atomic');
    if (bi < 0n) throw new Error('to_human() atomic must be non-negative');
    return decimalToString({ kind: 'decimal', int: bi, scale: decimals });
  },

  mul_div: (args) => {
    const [a, b, denom] = args;
    const aa = coerceBigInt(a, 'mul_div.a');
    const bb = coerceBigInt(b, 'mul_div.b');
    const dd = coerceBigInt(denom, 'mul_div.denom');
    if (aa < 0n || bb < 0n || dd < 0n) throw new Error('mul_div() args must be non-negative');
    if (dd === 0n) throw new Error('mul_div() denom must be > 0');
    return (aa * bb) / dd;
  },

  pow: (args) => {
    const [base, exp] = args;
    const b = coerceBigInt(base, 'pow.base');
    const e = coerceBigInt(exp, 'pow.exp');
    if (e < 0n) throw new Error('pow() exponent must be non-negative');
    if (e > 10_000n) throw new Error('pow() exponent too large');
    let out = 1n;
    for (let i = 0n; i < e; i++) out *= b;
    return out;
  },
};

export class Evaluator {
  private parser = new Parser();
  private customFunctions: Record<string, CELFunction> = {};

  /**
   * Register a custom function
   */
  registerFunction(name: string, fn: CELFunction): void {
    this.customFunctions[name] = fn;
  }

  /**
   * Evaluate a CEL expression
   */
  evaluate(expression: string, context: CELContext = {}): CELValue {
    const ast = this.parser.parse(expression);
    return this.evalNode(ast, context);
  }

  private evalNode(node: ASTNode, ctx: CELContext): CELValue {
    switch (node.type) {
      case 'Literal':
        return node.value as CELValue;

      case 'Identifier':
        if (!(node.name in ctx)) {
          throw new Error(`Undefined variable: ${node.name}`);
        }
        return ctx[node.name];

      case 'Binary':
        return this.evalBinary(node, ctx);

      case 'Unary':
        return this.evalUnary(node, ctx);

      case 'Member':
        return this.evalMember(node, ctx);

      case 'Index':
        return this.evalIndex(node, ctx);

      case 'Call':
        return this.evalCall(node, ctx);

      case 'Ternary':
        return this.evalTernary(node, ctx);

      case 'List':
        return node.elements.map((el) => this.evalNode(el, ctx));

      case 'Map': {
        const result: Record<string, CELValue> = {};
        for (const entry of node.entries) {
          const key = this.evalNode(entry.key, ctx);
          if (typeof key !== 'string') {
            throw new Error('Map keys must be strings');
          }
          result[key] = this.evalNode(entry.value, ctx);
        }
        return result;
      }

      default:
        throw new Error(`Unknown node type: ${(node as ASTNode).type}`);
    }
  }

  private evalBinary(
    node: { operator: string; left: ASTNode; right: ASTNode },
    ctx: CELContext
  ): CELValue {
    // Short-circuit evaluation for && and ||
    if (node.operator === '&&') {
      const left = this.evalNode(node.left, ctx);
      if (!left) return false;
      return Boolean(this.evalNode(node.right, ctx));
    }

    if (node.operator === '||') {
      const left = this.evalNode(node.left, ctx);
      if (left) return true;
      return Boolean(this.evalNode(node.right, ctx));
    }

    const left = this.evalNode(node.left, ctx);
    const right = this.evalNode(node.right, ctx);

    switch (node.operator) {
      case '+':
        if (typeof left === 'string' || typeof right === 'string') {
          return String(left) + String(right);
        }
        if (isNumeric(left) && isNumeric(right)) return normalizeNumeric(decimalAdd(left, right));
        if (Array.isArray(left) && Array.isArray(right)) {
          return [...left, ...right];
        }
        throw new Error(`Cannot add ${typeof left} and ${typeof right}`);

      case '-':
        if (isNumeric(left) && isNumeric(right)) return normalizeNumeric(decimalSub(left, right));
        throw new Error('Subtraction requires numeric');

      case '*':
        if (isNumeric(left) && isNumeric(right)) return normalizeNumeric(decimalMul(left, right));
        throw new Error('Multiplication requires numeric');

      case '/':
        if (isNumeric(left) && isNumeric(right)) {
          if (typeof left === 'bigint' && typeof right === 'bigint') {
            if (right === 0n) throw new Error('Division by zero');
            if (left % right === 0n) return left / right;
          }
          return normalizeNumeric(decimalDiv(left, right));
        }
        throw new Error('Division requires numeric');

      case '%':
        if (typeof left === 'bigint' && typeof right === 'bigint') {
          if (right === 0n) throw new Error('Division by zero');
          return left % right;
        }
        throw new Error('Modulo requires int');

      case '==':
        return this.celEqual(left, right);

      case '!=':
        return !this.celEqual(left, right);

      case '<':
        if (isNumeric(left) && isNumeric(right)) return numericCompare(left, right) < 0;
        if (typeof left === 'string' && typeof right === 'string') {
          return left < right;
        }
        throw new Error('Comparison requires same types');

      case '<=':
        if (isNumeric(left) && isNumeric(right)) return numericCompare(left, right) <= 0;
        if (typeof left === 'string' && typeof right === 'string') {
          return left <= right;
        }
        throw new Error('Comparison requires same types');

      case '>':
        if (isNumeric(left) && isNumeric(right)) return numericCompare(left, right) > 0;
        if (typeof left === 'string' && typeof right === 'string') {
          return left > right;
        }
        throw new Error('Comparison requires same types');

      case '>=':
        if (isNumeric(left) && isNumeric(right)) return numericCompare(left, right) >= 0;
        if (typeof left === 'string' && typeof right === 'string') {
          return left >= right;
        }
        throw new Error('Comparison requires same types');

      case 'in':
        if (Array.isArray(right)) {
          return right.some((item) => this.celEqual(left, item));
        }
        if (typeof right === 'string' && typeof left === 'string') {
          return right.includes(left);
        }
        if (right && typeof right === 'object' && typeof left === 'string') {
          return left in (right as Record<string, unknown>);
        }
        throw new Error(`'in' operator requires list, string, or map on right side`);

      default:
        throw new Error(`Unknown operator: ${node.operator}`);
    }
  }

  private evalUnary(
    node: { operator: string; operand: ASTNode },
    ctx: CELContext
  ): CELValue {
    const operand = this.evalNode(node.operand, ctx);

    switch (node.operator) {
      case '!':
        return !operand;

      case '-':
        if (typeof operand === 'bigint') return -operand;
        if (isCELDecimal(operand)) return normalizeNumeric({ kind: 'decimal', int: -operand.int, scale: operand.scale });
        throw new Error('Unary minus requires numeric');

      default:
        throw new Error(`Unknown unary operator: ${node.operator}`);
    }
  }

  private evalMember(
    node: { object: ASTNode; property: string },
    ctx: CELContext
  ): CELValue {
    const obj = this.evalNode(node.object, ctx);

    if (obj === null || obj === undefined) {
      throw new Error(`Cannot access property '${node.property}' of ${obj}`);
    }

    if (typeof obj !== 'object' || Array.isArray(obj)) {
      throw new Error(`Cannot access property '${node.property}' on non-object`);
    }

    const value = (obj as Record<string, CELValue>)[node.property];
    return value ?? null;
  }

  private evalIndex(
    node: { object: ASTNode; index: ASTNode },
    ctx: CELContext
  ): CELValue {
    const obj = this.evalNode(node.object, ctx);
    const index = this.evalNode(node.index, ctx);

    if (Array.isArray(obj)) {
      const i = toIndex(index);
      return obj[i] ?? null;
    }

    if (typeof obj === 'string') {
      const i = toIndex(index);
      return obj[i] ?? null;
    }

    if (obj && typeof obj === 'object') {
      if (typeof index !== 'string') {
        throw new Error('Map key must be a string');
      }
      return (obj as Record<string, CELValue>)[index] ?? null;
    }

    throw new Error('Indexing requires list, string, or map');
  }

  private evalCall(
    node: { callee: ASTNode; args: ASTNode[] },
    ctx: CELContext
  ): CELValue {
    // Get function name
    let fnName: string;
    let receiver: CELValue | null = null;

    if (node.callee.type === 'Identifier') {
      fnName = node.callee.name;
    } else if (node.callee.type === 'Member') {
      receiver = this.evalNode(node.callee.object, ctx);
      fnName = node.callee.property;
    } else {
      throw new Error('Invalid function call');
    }

    // Evaluate arguments
    const args = node.args.map((arg) => this.evalNode(arg, ctx));

    // If there's a receiver, prepend it to args
    if (receiver !== null) {
      args.unshift(receiver);
    }

    // Look up function
    const fn = this.customFunctions[fnName] ?? BUILTINS[fnName];
    if (!fn) {
      throw new Error(`Unknown function: ${fnName}`);
    }

    return fn(args, ctx);
  }

  private evalTernary(
    node: { condition: ASTNode; consequent: ASTNode; alternate: ASTNode },
    ctx: CELContext
  ): CELValue {
    const condition = this.evalNode(node.condition, ctx);
    return condition
      ? this.evalNode(node.consequent, ctx)
      : this.evalNode(node.alternate, ctx);
  }

  private deepEqual(a: CELValue, b: CELValue): boolean {
    if (a === b) return true;
    if (typeof a !== typeof b) return false;
    if (a === null || b === null) return a === b;

    if (Array.isArray(a) && Array.isArray(b)) {
      if (a.length !== b.length) return false;
      return a.every((item, i) => this.deepEqual(item, b[i]));
    }

    if (typeof a === 'object' && typeof b === 'object') {
      if (isCELDecimal(a) && isCELDecimal(b)) {
        const na = normalizeDecimal(a);
        const nb = normalizeDecimal(b);
        return na.int === nb.int && na.scale === nb.scale;
      }
      const keysA = Object.keys(a);
      const keysB = Object.keys(b);
      if (keysA.length !== keysB.length) return false;
      return keysA.every((key) =>
        this.deepEqual(
          (a as Record<string, CELValue>)[key],
          (b as Record<string, CELValue>)[key]
        )
      );
    }

    return false;
  }

  private celEqual(a: CELValue, b: CELValue): boolean {
    if (isNumeric(a) && isNumeric(b)) return numericCompare(a, b) === 0;
    return this.deepEqual(a, b);
  }
}

/**
 * Convenience function to evaluate a CEL expression
 */
export function evaluateCEL(expression: string, context: CELContext = {}): CELValue {
  const evaluator = new Evaluator();
  return evaluator.evaluate(expression, context);
}

function isNumeric(v: CELValue): v is bigint | CELDecimal {
  return typeof v === 'bigint' || isCELDecimal(v);
}

function numericCompare(a: bigint | CELDecimal, b: bigint | CELDecimal): -1 | 0 | 1 {
  return decimalCompare(a, b);
}

function normalizeNumeric(v: bigint | CELDecimal): bigint | CELDecimal {
  if (typeof v === 'bigint') return v;
  const n = normalizeDecimal(v);
  return n.scale === 0 ? n.int : n;
}

function truncDecimalToInt(d: CELDecimal): bigint {
  const n = normalizeDecimal(d);
  if (n.scale === 0) return n.int;
  return n.int / pow10(n.scale);
}

function getDecimals(asset: CELValue): number {
  if (typeof asset === 'bigint') {
    if (asset > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('decimals too large');
    return Number(asset);
  }
  if (typeof asset === 'string') {
    if (!/^\d+$/.test(asset)) throw new Error('decimals must be integer string');
    const n = Number(asset);
    if (!Number.isSafeInteger(n)) throw new Error('decimals too large');
    return n;
  }
  if (asset && typeof asset === 'object' && !Array.isArray(asset) && 'decimals' in asset) {
    const d = (asset as { decimals: unknown }).decimals;
    if (typeof d === 'number') {
      if (!Number.isSafeInteger(d)) throw new Error('decimals must be int');
      return d;
    }
    if (typeof d === 'bigint') {
      if (d > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('decimals too large');
      return Number(d);
    }
    if (typeof d === 'string') {
      if (!/^\d+$/.test(d)) throw new Error('decimals must be integer string');
      const n = Number(d);
      if (!Number.isSafeInteger(n)) throw new Error('decimals too large');
      return n;
    }
    throw new Error('decimals must be number/bigint/string');
  }
  throw new Error('to_atomic/to_human() second arg must be asset with decimals or decimal count');
}

function coerceAmountDecimal(amount: CELValue): CELDecimal {
  if (typeof amount === 'bigint') return { kind: 'decimal', int: amount, scale: 0 };
  if (isCELDecimal(amount)) return amount;
  if (typeof amount === 'string') return parseDecimalString(amount);
  throw new Error('to_atomic() first arg must be int/decimal/string');
}

function coerceBigInt(v: CELValue, path: string): bigint {
  if (typeof v === 'bigint') return v;
  if (isCELDecimal(v)) {
    const n = normalizeDecimal(v);
    if (n.scale !== 0) throw new Error(`${path} must be integer`);
    return n.int;
  }
  if (typeof v === 'string') {
    if (!/^-?\d+$/.test(v)) throw new Error(`${path} must be integer string`);
    return BigInt(v);
  }
  throw new Error(`${path} must be int/decimal/string`);
}

function toIndex(v: CELValue): number {
  if (typeof v === 'bigint') {
    if (v < 0n) throw new Error('Index must be non-negative');
    if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('Index too large');
    return Number(v);
  }
  if (isCELDecimal(v)) {
    const n = normalizeDecimal(v);
    if (n.scale !== 0) throw new Error('Index must be integer');
    return toIndex(n.int);
  }
  throw new Error('Index must be int');
}
