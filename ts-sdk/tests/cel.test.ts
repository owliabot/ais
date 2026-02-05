import { describe, it, expect } from 'vitest';
import { evaluateCEL, CELEvaluator } from '../src/index.js';

describe('CEL Evaluator', () => {
  describe('literals', () => {
    it('evaluates numbers', () => {
      expect(evaluateCEL('42')).toBe(42);
      expect(evaluateCEL('3.14')).toBe(3.14);
      expect(evaluateCEL('-5')).toBe(-5);
    });

    it('evaluates strings', () => {
      expect(evaluateCEL('"hello"')).toBe('hello');
      expect(evaluateCEL("'world'")).toBe('world');
      expect(evaluateCEL('"with\\"quote"')).toBe('with"quote');
    });

    it('evaluates booleans', () => {
      expect(evaluateCEL('true')).toBe(true);
      expect(evaluateCEL('false')).toBe(false);
    });

    it('evaluates null', () => {
      expect(evaluateCEL('null')).toBe(null);
    });

    it('evaluates lists', () => {
      expect(evaluateCEL('[1, 2, 3]')).toEqual([1, 2, 3]);
      expect(evaluateCEL('[]')).toEqual([]);
      expect(evaluateCEL('["a", "b"]')).toEqual(['a', 'b']);
    });
  });

  describe('arithmetic', () => {
    it('evaluates addition', () => {
      expect(evaluateCEL('1 + 2')).toBe(3);
      expect(evaluateCEL('"a" + "b"')).toBe('ab');
    });

    it('evaluates subtraction', () => {
      expect(evaluateCEL('5 - 3')).toBe(2);
    });

    it('evaluates multiplication', () => {
      expect(evaluateCEL('4 * 3')).toBe(12);
    });

    it('evaluates division', () => {
      expect(evaluateCEL('10 / 4')).toBe(2.5);
    });

    it('evaluates modulo', () => {
      expect(evaluateCEL('10 % 3')).toBe(1);
    });

    it('respects operator precedence', () => {
      expect(evaluateCEL('2 + 3 * 4')).toBe(14);
      expect(evaluateCEL('(2 + 3) * 4')).toBe(20);
    });
  });

  describe('comparison', () => {
    it('evaluates equality', () => {
      expect(evaluateCEL('1 == 1')).toBe(true);
      expect(evaluateCEL('1 == 2')).toBe(false);
      expect(evaluateCEL('"a" == "a"')).toBe(true);
      expect(evaluateCEL('[1, 2] == [1, 2]')).toBe(true);
    });

    it('evaluates inequality', () => {
      expect(evaluateCEL('1 != 2')).toBe(true);
      expect(evaluateCEL('1 != 1')).toBe(false);
    });

    it('evaluates less than', () => {
      expect(evaluateCEL('1 < 2')).toBe(true);
      expect(evaluateCEL('2 < 1')).toBe(false);
    });

    it('evaluates less than or equal', () => {
      expect(evaluateCEL('1 <= 1')).toBe(true);
      expect(evaluateCEL('1 <= 2')).toBe(true);
      expect(evaluateCEL('2 <= 1')).toBe(false);
    });

    it('evaluates greater than', () => {
      expect(evaluateCEL('2 > 1')).toBe(true);
      expect(evaluateCEL('1 > 2')).toBe(false);
    });

    it('evaluates greater than or equal', () => {
      expect(evaluateCEL('2 >= 2')).toBe(true);
      expect(evaluateCEL('2 >= 1')).toBe(true);
      expect(evaluateCEL('1 >= 2')).toBe(false);
    });
  });

  describe('logical operators', () => {
    it('evaluates AND', () => {
      expect(evaluateCEL('true && true')).toBe(true);
      expect(evaluateCEL('true && false')).toBe(false);
      expect(evaluateCEL('false && true')).toBe(false);
    });

    it('evaluates OR', () => {
      expect(evaluateCEL('true || false')).toBe(true);
      expect(evaluateCEL('false || true')).toBe(true);
      expect(evaluateCEL('false || false')).toBe(false);
    });

    it('evaluates NOT', () => {
      expect(evaluateCEL('!true')).toBe(false);
      expect(evaluateCEL('!false')).toBe(true);
    });

    it('short-circuits AND', () => {
      // Should not throw because second operand is not evaluated
      expect(evaluateCEL('false && undefined_var', { undefined_var: null })).toBe(false);
    });

    it('short-circuits OR', () => {
      expect(evaluateCEL('true || undefined_var', { undefined_var: null })).toBe(true);
    });
  });

  describe('ternary operator', () => {
    it('evaluates ternary', () => {
      expect(evaluateCEL('true ? 1 : 2')).toBe(1);
      expect(evaluateCEL('false ? 1 : 2')).toBe(2);
    });

    it('evaluates nested ternary', () => {
      expect(evaluateCEL('true ? (false ? 1 : 2) : 3')).toBe(2);
    });
  });

  describe('in operator', () => {
    it('checks list membership', () => {
      expect(evaluateCEL('1 in [1, 2, 3]')).toBe(true);
      expect(evaluateCEL('4 in [1, 2, 3]')).toBe(false);
    });

    it('checks string containment', () => {
      expect(evaluateCEL('"ell" in "hello"')).toBe(true);
      expect(evaluateCEL('"xyz" in "hello"')).toBe(false);
    });

    it('checks map key existence', () => {
      expect(evaluateCEL('"a" in obj', { obj: { a: 1, b: 2 } })).toBe(true);
      expect(evaluateCEL('"c" in obj', { obj: { a: 1, b: 2 } })).toBe(false);
    });
  });

  describe('variables', () => {
    it('accesses simple variables', () => {
      expect(evaluateCEL('x', { x: 42 })).toBe(42);
      expect(evaluateCEL('name', { name: 'Alice' })).toBe('Alice');
    });

    it('accesses nested properties', () => {
      expect(evaluateCEL('user.name', { user: { name: 'Bob' } })).toBe('Bob');
      expect(evaluateCEL('a.b.c', { a: { b: { c: 123 } } })).toBe(123);
    });

    it('accesses array elements', () => {
      expect(evaluateCEL('arr[0]', { arr: [10, 20, 30] })).toBe(10);
      expect(evaluateCEL('arr[2]', { arr: [10, 20, 30] })).toBe(30);
    });

    it('accesses map values', () => {
      expect(evaluateCEL('obj["key"]', { obj: { key: 'value' } })).toBe('value');
    });

    it('throws on undefined variable', () => {
      expect(() => evaluateCEL('undefined_var')).toThrow('Undefined variable');
    });
  });

  describe('built-in functions', () => {
    it('size() returns length', () => {
      expect(evaluateCEL('size("hello")')).toBe(5);
      expect(evaluateCEL('size([1, 2, 3])')).toBe(3);
      expect(evaluateCEL('size(obj)', { obj: { a: 1, b: 2 } })).toBe(2);
    });

    it('contains() checks substring', () => {
      expect(evaluateCEL('contains("hello", "ell")')).toBe(true);
      expect(evaluateCEL('contains("hello", "xyz")')).toBe(false);
    });

    it('startsWith() checks prefix', () => {
      expect(evaluateCEL('startsWith("hello", "hel")')).toBe(true);
      expect(evaluateCEL('startsWith("hello", "lo")')).toBe(false);
    });

    it('endsWith() checks suffix', () => {
      expect(evaluateCEL('endsWith("hello", "lo")')).toBe(true);
      expect(evaluateCEL('endsWith("hello", "hel")')).toBe(false);
    });

    it('matches() tests regex', () => {
      expect(evaluateCEL('matches("hello123", "[a-z]+\\\\d+")')).toBe(true);
      expect(evaluateCEL('matches("hello", "\\\\d+")')).toBe(false);
    });

    it('lower() converts to lowercase', () => {
      expect(evaluateCEL('lower("HELLO")')).toBe('hello');
    });

    it('upper() converts to uppercase', () => {
      expect(evaluateCEL('upper("hello")')).toBe('HELLO');
    });

    it('trim() removes whitespace', () => {
      expect(evaluateCEL('trim("  hello  ")')).toBe('hello');
    });

    it('int() converts to integer', () => {
      expect(evaluateCEL('int(3.7)')).toBe(3);
      expect(evaluateCEL('int("42")')).toBe(42);
    });

    it('string() converts to string', () => {
      expect(evaluateCEL('string(42)')).toBe('42');
      expect(evaluateCEL('string(true)')).toBe('true');
    });

    it('type() returns type name', () => {
      expect(evaluateCEL('type(42)')).toBe('number');
      expect(evaluateCEL('type("hello")')).toBe('string');
      expect(evaluateCEL('type(true)')).toBe('boolean');
      expect(evaluateCEL('type(null)')).toBe('null');
      expect(evaluateCEL('type([1,2])')).toBe('list');
    });

    it('abs() returns absolute value', () => {
      expect(evaluateCEL('abs(-5)')).toBe(5);
      expect(evaluateCEL('abs(5)')).toBe(5);
    });

    it('min() returns minimum', () => {
      expect(evaluateCEL('min(3, 1, 2)')).toBe(1);
    });

    it('max() returns maximum', () => {
      expect(evaluateCEL('max(3, 1, 2)')).toBe(3);
    });

    it('floor() floors to integer', () => {
      expect(evaluateCEL('floor(3.7)')).toBe(3);
      expect(evaluateCEL('floor(3.2)')).toBe(3);
      expect(evaluateCEL('floor(-3.2)')).toBe(-4);
    });

    it('ceil() ceils to integer', () => {
      expect(evaluateCEL('ceil(3.2)')).toBe(4);
      expect(evaluateCEL('ceil(3.7)')).toBe(4);
      expect(evaluateCEL('ceil(-3.7)')).toBe(-3);
    });

    it('round() rounds to nearest integer', () => {
      expect(evaluateCEL('round(3.4)')).toBe(3);
      expect(evaluateCEL('round(3.5)')).toBe(4);
      expect(evaluateCEL('round(3.6)')).toBe(4);
    });

    it('pow() computes power', () => {
      expect(evaluateCEL('pow(2, 3)')).toBe(8);
      expect(evaluateCEL('pow(10, 2)')).toBe(100);
      expect(evaluateCEL('pow(10, 0)')).toBe(1);
    });
  });

  describe('AIS token conversion functions', () => {
    it('to_atomic() converts human amount with decimals number', () => {
      // 1.5 tokens with 18 decimals
      expect(evaluateCEL('to_atomic(1.5, 18)')).toBe('1500000000000000000');
      // 100 USDC with 6 decimals
      expect(evaluateCEL('to_atomic(100, 6)')).toBe('100000000');
      // 0.001 ETH
      expect(evaluateCEL('to_atomic(0.001, 18)')).toBe('1000000000000000');
    });

    it('to_atomic() converts human amount with asset object', () => {
      const ctx = {
        token: { decimals: 18, symbol: 'WETH' },
        amount: 2.5,
      };
      expect(evaluateCEL('to_atomic(amount, token)', ctx)).toBe('2500000000000000000');
    });

    it('to_atomic() handles string amounts', () => {
      expect(evaluateCEL('to_atomic("1.0", 18)')).toBe('1000000000000000000');
    });

    it('to_human() converts atomic amount to human', () => {
      // 1e18 wei to ETH
      expect(evaluateCEL('to_human(1000000000000000000, 18)')).toBe(1);
      // 1e6 to USDC
      expect(evaluateCEL('to_human(1000000, 6)')).toBe(1);
      // 2.5 ETH in wei
      expect(evaluateCEL('to_human(2500000000000000000, 18)')).toBe(2.5);
    });

    it('to_human() works with asset object', () => {
      const ctx = {
        token: { decimals: 6, symbol: 'USDC' },
        atomic: 50000000, // 50 USDC
      };
      expect(evaluateCEL('to_human(atomic, token)', ctx)).toBe(50);
    });

    it('to_human() handles string atomic amounts', () => {
      expect(evaluateCEL('to_human("1000000000000000000", 18)')).toBe(1);
    });

    it('calculates min_out with slippage (AIS pattern)', () => {
      const ctx = {
        quote_amount_out_atomic: 1000000000000000000, // 1 token
        slippage_bps: 50, // 0.5%
      };
      // floor(amount * (1 - slippage/10000))
      const result = evaluateCEL(
        'floor(quote_amount_out_atomic * (1.0 - slippage_bps / 10000.0))',
        ctx
      );
      expect(result).toBe(995000000000000000); // 0.995 token
    });
  });

  describe('method-style function calls', () => {
    it('calls methods on strings', () => {
      expect(evaluateCEL('"hello".contains("ell")')).toBe(true);
      expect(evaluateCEL('"hello".startsWith("he")')).toBe(true);
      expect(evaluateCEL('"HELLO".lower()')).toBe('hello');
    });

    it('calls methods on lists', () => {
      expect(evaluateCEL('[1, 2, 3].size()')).toBe(3);
    });
  });

  describe('custom functions', () => {
    it('allows registering custom functions', () => {
      const evaluator = new CELEvaluator();
      evaluator.registerFunction('double', (args) => {
        const n = args[0];
        if (typeof n !== 'number') throw new Error('Expected number');
        return n * 2;
      });

      expect(evaluator.evaluate('double(21)')).toBe(42);
    });
  });

  describe('complex expressions', () => {
    it('evaluates complex boolean expressions', () => {
      const ctx = {
        user: { age: 25, verified: true },
        minAge: 18,
      };
      expect(evaluateCEL('user.age >= minAge && user.verified', ctx)).toBe(true);
    });

    it('evaluates AIS-style conditions', () => {
      const ctx = {
        inputs: { amount: 1000, slippage_bps: 50 },
        constraints: { max_slippage_bps: 100 },
      };
      expect(
        evaluateCEL('inputs.slippage_bps <= constraints.max_slippage_bps', ctx)
      ).toBe(true);
    });

    it('evaluates node output references', () => {
      const ctx = {
        nodes: {
          approve: { outputs: { success: true } },
          quote: { outputs: { amountOut: 950 } },
        },
      };
      expect(evaluateCEL('nodes.approve.outputs.success == true', ctx)).toBe(true);
      expect(evaluateCEL('nodes.quote.outputs.amountOut > 900', ctx)).toBe(true);
    });

    it('evaluates token allowlist check', () => {
      const ctx = {
        token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
        allowlist: [
          '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
          '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48',
        ],
      };
      expect(evaluateCEL('token in allowlist', ctx)).toBe(true);
    });

    it('evaluates calculated field expressions', () => {
      const ctx = {
        amount_in: 1000000000000000000,
        quote_out: 950000000,
        slippage_bps: 50,
      };
      expect(
        evaluateCEL('quote_out * (10000 - slippage_bps) / 10000', ctx)
      ).toBe(945250000);
    });
  });

  describe('error handling', () => {
    it('throws on syntax errors', () => {
      expect(() => evaluateCEL('1 +')).toThrow();
      expect(() => evaluateCEL('((1 + 2)')).toThrow();
    });

    it('throws on unknown functions', () => {
      expect(() => evaluateCEL('unknownFunc(1)')).toThrow('Unknown function');
    });

    it('throws on type errors', () => {
      expect(() => evaluateCEL('"a" - "b"')).toThrow();
      expect(() => evaluateCEL('1 / 0')).toThrow('Division by zero');
    });
  });
});
