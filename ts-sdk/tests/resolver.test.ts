import { describe, it, expect, beforeEach } from 'vitest';
import {
  createContext,
  registerProtocol,
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  hasExpressions,
  extractExpressions,
  resolveExpression,
  resolveExpressionString,
  setVariable,
  setQueryResult,
  parseProtocolSpec,
  type ResolverContext,
} from '../src/index.js';

const SAMPLE_PROTOCOL = `
ais_version: "1.0"
type: protocol
protocol:
  name: uniswap-v3
  version: "1.0.0"
  chain_id: 1
  addresses:
    router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
    factory: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
queries:
  - name: get_pool
    contract: factory
    method: getPool
    inputs:
      - name: token0
        type: address
    outputs:
      - name: pool
        type: address
actions:
  - name: swap_exact_in
    contract: router
    method: exactInputSingle
    inputs:
      - name: tokenIn
        type: address
      - name: amountIn
        type: uint256
`;

describe('ResolverContext', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
  });

  it('creates empty context', () => {
    expect(ctx.protocols.size).toBe(0);
    expect(Object.keys(ctx.variables)).toHaveLength(0);
    expect(ctx.queryResults.size).toBe(0);
  });

  it('registers protocols', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    registerProtocol(ctx, spec);
    expect(ctx.protocols.has('uniswap-v3')).toBe(true);
  });
});

describe('resolveProtocolRef', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
  });

  it('resolves protocol by name', () => {
    const spec = resolveProtocolRef(ctx, 'uniswap-v3');
    expect(spec).not.toBeNull();
    expect(spec?.protocol.name).toBe('uniswap-v3');
  });

  it('resolves protocol with version', () => {
    const spec = resolveProtocolRef(ctx, 'uniswap-v3@1.0.0');
    expect(spec).not.toBeNull();
  });

  it('returns null for wrong version', () => {
    const spec = resolveProtocolRef(ctx, 'uniswap-v3@2.0.0');
    expect(spec).toBeNull();
  });

  it('returns null for unknown protocol', () => {
    const spec = resolveProtocolRef(ctx, 'unknown');
    expect(spec).toBeNull();
  });
});

describe('resolveAction', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
  });

  it('resolves action by reference', () => {
    const result = resolveAction(ctx, 'uniswap-v3/swap_exact_in');
    expect(result).not.toBeNull();
    expect(result?.action.name).toBe('swap_exact_in');
    expect(result?.action.method).toBe('exactInputSingle');
  });

  it('returns null for unknown action', () => {
    const result = resolveAction(ctx, 'uniswap-v3/unknown');
    expect(result).toBeNull();
  });
});

describe('resolveQuery', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
  });

  it('resolves query by reference', () => {
    const result = resolveQuery(ctx, 'uniswap-v3/get_pool');
    expect(result).not.toBeNull();
    expect(result?.query.name).toBe('get_pool');
  });
});

describe('expression handling', () => {
  it('detects expressions in strings', () => {
    expect(hasExpressions('${input.amount}')).toBe(true);
    expect(hasExpressions('plain text')).toBe(false);
    expect(hasExpressions('${a} and ${b}')).toBe(true);
  });

  it('extracts expression references', () => {
    const exprs = extractExpressions('${input.amount} + ${query.pool.fee}');
    expect(exprs).toEqual(['input.amount', 'query.pool.fee']);
  });
});

describe('resolveExpression', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
  });

  it('resolves input variables', () => {
    setVariable(ctx, 'amount', 1000);
    expect(resolveExpression('input.amount', ctx)).toBe(1000);
    expect(resolveExpression('inputs.amount', ctx)).toBe(1000);
  });

  it('resolves query results', () => {
    setQueryResult(ctx, 'get_pool', { pool: '0xabc', fee: 3000 });
    expect(resolveExpression('query.get_pool.pool', ctx)).toBe('0xabc');
    expect(resolveExpression('query.get_pool.fee', ctx)).toBe(3000);
  });

  it('resolves step outputs', () => {
    setVariable(ctx, 'step.approve', { success: true, tx: '0x123' });
    expect(resolveExpression('step.approve.success', ctx)).toBe(true);
    expect(resolveExpression('step.approve.tx', ctx)).toBe('0x123');
  });

  it('resolves addresses', () => {
    const router = resolveExpression('address.router', ctx);
    expect(router).toBe('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');
  });

  it('returns undefined for missing references', () => {
    expect(resolveExpression('input.missing', ctx)).toBeUndefined();
  });
});

describe('resolveExpressionString', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
    setVariable(ctx, 'amount', 1000);
    setVariable(ctx, 'token', '0xWETH');
  });

  it('resolves all expressions in a string', () => {
    const result = resolveExpressionString(
      'Swap ${input.amount} of ${input.token}',
      ctx
    );
    expect(result).toBe('Swap 1000 of 0xWETH');
  });

  it('preserves unresolved expressions', () => {
    const result = resolveExpressionString('${input.missing}', ctx);
    expect(result).toBe('${input.missing}');
  });

  it('handles mixed content', () => {
    const result = resolveExpressionString(
      'Amount: ${input.amount}, Router: ${address.router}',
      ctx
    );
    expect(result).toContain('1000');
    expect(result).toContain('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');
  });
});
