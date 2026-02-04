import { describe, it, expect, beforeEach } from 'vitest';
import {
  createContext,
  registerProtocol,
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  parseSkillRef,
  getContractAddress,
  getSupportedChains,
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
schema: "ais/1.0"
meta:
  protocol: uniswap-v3
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
      factory: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
  - chain: "eip155:8453"
    contracts:
      router: "0x2626664c2603336E57B271c5C0b26F421741e481"
queries:
  get_pool:
    contract: factory
    method: getPool
    params:
      - name: token0
        type: address
    outputs:
      - name: pool
        type: address
actions:
  swap_exact_in:
    contract: router
    method: exactInputSingle
    params:
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

describe('parseSkillRef', () => {
  it('parses protocol only', () => {
    const ref = parseSkillRef('uniswap-v3');
    expect(ref.protocol).toBe('uniswap-v3');
    expect(ref.version).toBeUndefined();
  });

  it('parses protocol with version', () => {
    const ref = parseSkillRef('uniswap-v3@1.0.0');
    expect(ref.protocol).toBe('uniswap-v3');
    expect(ref.version).toBe('1.0.0');
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
    expect(spec?.meta.protocol).toBe('uniswap-v3');
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
    expect(result?.actionId).toBe('swap_exact_in');
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
    expect(result?.queryId).toBe('get_pool');
  });
});

describe('getContractAddress', () => {
  it('gets contract address for chain', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    const addr = getContractAddress(spec, 'eip155:1', 'router');
    expect(addr).toBe('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');
  });

  it('returns null for unknown chain', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    const addr = getContractAddress(spec, 'eip155:999', 'router');
    expect(addr).toBeNull();
  });

  it('returns null for unknown contract', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    const addr = getContractAddress(spec, 'eip155:1', 'unknown');
    expect(addr).toBeNull();
  });
});

describe('getSupportedChains', () => {
  it('returns all supported chains', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    const chains = getSupportedChains(spec);
    expect(chains).toContain('eip155:1');
    expect(chains).toContain('eip155:8453');
  });
});

describe('expression handling', () => {
  it('detects expressions in strings', () => {
    expect(hasExpressions('${inputs.amount}')).toBe(true);
    expect(hasExpressions('plain text')).toBe(false);
    expect(hasExpressions('${a} and ${b}')).toBe(true);
  });

  it('extracts expression references', () => {
    const exprs = extractExpressions('${inputs.amount} + ${nodes.pool.outputs.fee}');
    expect(exprs).toEqual(['inputs.amount', 'nodes.pool.outputs.fee']);
  });
});

describe('resolveExpression', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    registerProtocol(ctx, parseProtocolSpec(SAMPLE_PROTOCOL));
  });

  it('resolves input variables', () => {
    setVariable(ctx, 'inputs.amount', 1000);
    expect(resolveExpression('inputs.amount', ctx)).toBe(1000);
  });

  it('resolves node outputs', () => {
    setVariable(ctx, 'nodes.get_pool', { outputs: { pool: '0xabc', fee: 3000 } });
    expect(resolveExpression('nodes.get_pool.outputs.pool', ctx)).toBe('0xabc');
    expect(resolveExpression('nodes.get_pool.outputs.fee', ctx)).toBe(3000);
  });

  it('resolves ctx variables', () => {
    setVariable(ctx, 'ctx.chain', 'eip155:1');
    setVariable(ctx, 'ctx.sender', '0xuser');
    expect(resolveExpression('ctx.chain', ctx)).toBe('eip155:1');
    expect(resolveExpression('ctx.sender', ctx)).toBe('0xuser');
  });

  it('resolves query results (legacy)', () => {
    setQueryResult(ctx, 'get_pool', { pool: '0xabc', fee: 3000 });
    expect(resolveExpression('query.get_pool.pool', ctx)).toBe('0xabc');
  });

  it('returns undefined for missing references', () => {
    expect(resolveExpression('inputs.missing', ctx)).toBeUndefined();
  });
});

describe('resolveExpressionString', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
    setVariable(ctx, 'inputs.amount', 1000);
    setVariable(ctx, 'inputs.token', '0xWETH');
  });

  it('resolves all expressions in a string', () => {
    const result = resolveExpressionString(
      'Swap ${inputs.amount} of ${inputs.token}',
      ctx
    );
    expect(result).toBe('Swap 1000 of 0xWETH');
  });

  it('preserves unresolved expressions', () => {
    const result = resolveExpressionString('${inputs.missing}', ctx);
    expect(result).toBe('${inputs.missing}');
  });
});
