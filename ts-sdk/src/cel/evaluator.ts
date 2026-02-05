/**
 * CEL Evaluator - evaluate AST against a context
 */

import { Parser, ASTNode } from './parser.js';

export type CELValue =
  | string
  | number
  | boolean
  | null
  | CELValue[]
  | { [key: string]: CELValue };

export type CELContext = Record<string, CELValue>;

// Built-in functions
type CELFunction = (args: CELValue[], ctx: CELContext) => CELValue;

const BUILTINS: Record<string, CELFunction> = {
  // Size functions
  size: (args) => {
    const val = args[0];
    if (typeof val === 'string') return val.length;
    if (Array.isArray(val)) return val.length;
    if (val && typeof val === 'object') return Object.keys(val).length;
    throw new Error('size() requires string, list, or map');
  },

  // String functions
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

  // Type functions
  int: (args) => {
    const val = args[0];
    if (typeof val === 'number') return Math.floor(val);
    if (typeof val === 'string') return parseInt(val, 10);
    throw new Error('int() requires number or string');
  },

  uint: (args) => {
    const val = args[0];
    if (typeof val === 'number') return Math.abs(Math.floor(val));
    if (typeof val === 'string') return Math.abs(parseInt(val, 10));
    throw new Error('uint() requires number or string');
  },

  double: (args) => {
    const val = args[0];
    if (typeof val === 'number') return val;
    if (typeof val === 'string') return parseFloat(val);
    throw new Error('double() requires number or string');
  },

  string: (args) => {
    const val = args[0];
    return String(val);
  },

  bool: (args) => {
    const val = args[0];
    if (typeof val === 'boolean') return val;
    if (typeof val === 'string') return val === 'true';
    if (typeof val === 'number') return val !== 0;
    return Boolean(val);
  },

  // Type checking
  type: (args) => {
    const val = args[0];
    if (val === null) return 'null';
    if (Array.isArray(val)) return 'list';
    return typeof val;
  },

  // List functions
  exists: (args) => {
    const [list] = args;
    if (!Array.isArray(list)) throw new Error('exists() requires list');
    // Simplified: just check if any element is truthy
    return list.some((item) => Boolean(item));
  },

  all: (args) => {
    const [list] = args;
    if (!Array.isArray(list)) throw new Error('all() requires list');
    return list.every((item) => Boolean(item));
  },

  // Math functions
  abs: (args) => {
    const val = args[0];
    if (typeof val !== 'number') throw new Error('abs() requires number');
    return Math.abs(val);
  },

  min: (args) => {
    const nums = args.filter((a): a is number => typeof a === 'number');
    if (nums.length === 0) throw new Error('min() requires numbers');
    return Math.min(...nums);
  },

  max: (args) => {
    const nums = args.filter((a): a is number => typeof a === 'number');
    if (nums.length === 0) throw new Error('max() requires numbers');
    return Math.max(...nums);
  },

  floor: (args) => {
    const val = args[0];
    if (typeof val !== 'number') throw new Error('floor() requires number');
    return Math.floor(val);
  },

  ceil: (args) => {
    const val = args[0];
    if (typeof val !== 'number') throw new Error('ceil() requires number');
    return Math.ceil(val);
  },

  round: (args) => {
    const val = args[0];
    if (typeof val !== 'number') throw new Error('round() requires number');
    return Math.round(val);
  },

  // AIS-specific functions for token amount conversion
  // to_atomic(amount, asset) - Convert human amount to atomic using asset decimals
  // asset should have a 'decimals' property
  to_atomic: (args) => {
    const [amount, asset] = args;
    
    if (typeof amount !== 'number' && typeof amount !== 'string') {
      throw new Error('to_atomic() first arg must be number or string amount');
    }
    
    // Get decimals from asset
    let decimals: number;
    if (typeof asset === 'number') {
      decimals = asset;
    } else if (asset && typeof asset === 'object' && 'decimals' in asset) {
      decimals = (asset as { decimals: number }).decimals;
    } else {
      throw new Error('to_atomic() second arg must be asset with decimals or decimal count');
    }
    
    const numAmount = typeof amount === 'string' ? parseFloat(amount) : amount;
    
    // Use string math to avoid floating point issues for large numbers
    // For precision, we multiply and then truncate
    const multiplier = Math.pow(10, decimals);
    const result = Math.floor(numAmount * multiplier);
    
    // Return as string for uint256 compatibility
    return result.toString();
  },

  // to_human(atomic, asset) - Convert atomic amount to human readable
  to_human: (args) => {
    const [atomic, asset] = args;
    
    if (typeof atomic !== 'number' && typeof atomic !== 'string') {
      throw new Error('to_human() first arg must be number or string atomic amount');
    }
    
    // Get decimals from asset
    let decimals: number;
    if (typeof asset === 'number') {
      decimals = asset;
    } else if (asset && typeof asset === 'object' && 'decimals' in asset) {
      decimals = (asset as { decimals: number }).decimals;
    } else {
      throw new Error('to_human() second arg must be asset with decimals or decimal count');
    }
    
    const numAtomic = typeof atomic === 'string' ? parseFloat(atomic) : atomic;
    const divisor = Math.pow(10, decimals);
    
    return numAtomic / divisor;
  },

  // pow(base, exponent) - Power function
  pow: (args) => {
    const [base, exp] = args;
    if (typeof base !== 'number' || typeof exp !== 'number') {
      throw new Error('pow() requires two numbers');
    }
    return Math.pow(base, exp);
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
        if (typeof left === 'number' && typeof right === 'number') {
          return left + right;
        }
        if (Array.isArray(left) && Array.isArray(right)) {
          return [...left, ...right];
        }
        throw new Error(`Cannot add ${typeof left} and ${typeof right}`);

      case '-':
        if (typeof left === 'number' && typeof right === 'number') {
          return left - right;
        }
        throw new Error('Subtraction requires numbers');

      case '*':
        if (typeof left === 'number' && typeof right === 'number') {
          return left * right;
        }
        throw new Error('Multiplication requires numbers');

      case '/':
        if (typeof left === 'number' && typeof right === 'number') {
          if (right === 0) throw new Error('Division by zero');
          return left / right;
        }
        throw new Error('Division requires numbers');

      case '%':
        if (typeof left === 'number' && typeof right === 'number') {
          return left % right;
        }
        throw new Error('Modulo requires numbers');

      case '==':
        return this.deepEqual(left, right);

      case '!=':
        return !this.deepEqual(left, right);

      case '<':
        if (typeof left === 'number' && typeof right === 'number') {
          return left < right;
        }
        if (typeof left === 'string' && typeof right === 'string') {
          return left < right;
        }
        throw new Error('Comparison requires same types');

      case '<=':
        if (typeof left === 'number' && typeof right === 'number') {
          return left <= right;
        }
        if (typeof left === 'string' && typeof right === 'string') {
          return left <= right;
        }
        throw new Error('Comparison requires same types');

      case '>':
        if (typeof left === 'number' && typeof right === 'number') {
          return left > right;
        }
        if (typeof left === 'string' && typeof right === 'string') {
          return left > right;
        }
        throw new Error('Comparison requires same types');

      case '>=':
        if (typeof left === 'number' && typeof right === 'number') {
          return left >= right;
        }
        if (typeof left === 'string' && typeof right === 'string') {
          return left >= right;
        }
        throw new Error('Comparison requires same types');

      case 'in':
        if (Array.isArray(right)) {
          return right.some((item) => this.deepEqual(left, item));
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
        if (typeof operand === 'number') {
          return -operand;
        }
        throw new Error('Unary minus requires number');

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
      if (typeof index !== 'number') {
        throw new Error('List index must be a number');
      }
      return obj[index] ?? null;
    }

    if (typeof obj === 'string') {
      if (typeof index !== 'number') {
        throw new Error('String index must be a number');
      }
      return obj[index] ?? null;
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
}

/**
 * Convenience function to evaluate a CEL expression
 */
export function evaluateCEL(expression: string, context: CELContext = {}): CELValue {
  const evaluator = new Evaluator();
  return evaluator.evaluate(expression, context);
}
