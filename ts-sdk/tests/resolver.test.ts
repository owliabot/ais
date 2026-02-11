import { describe, it, expect, beforeEach } from 'vitest';
import {
  createContext,
  registerProtocol,
  resolveProtocolRef,
  resolveAction,
  resolveQuery,
  parseProtocolRef,
  getContractAddress,
  getSupportedChains,
  hasExpressions,
  extractExpressions,
  resolveExpression,
  resolveExpressionString,
  setRef,
  setQueryResult,
  parseProtocolSpec,
  type ResolverContext,
} from '../src/index.js';

const SAMPLE_PROTOCOL = `
schema: "ais/0.0.2"
meta:
  protocol: uniswap-v3
  version: "0.0.2"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
      factory: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
  - chain: "eip155:8453"
    contracts:
      router: "0x2626664c2603336E57B271c5C0b26F421741e481"
queries:
  get-pool:
    description: "Get pool address for token pair"
    params:
      - name: token0
        type: address
        description: "First token"
    returns:
      - name: pool
        type: address
    execution:
      "eip155:*":
        type: evm_read
        to: { lit: "0x1F98431c8aD98523631AE4a59f267346ea31F984" }
        abi:
          type: "function"
          name: "getPool"
          inputs:
            - { name: "token0", type: "address" }
          outputs:
            - { name: "pool", type: "address" }
        args:
          token0: { ref: "params.token0" }
actions:
  swap-exact-in:
    description: "Swap exact input amount"
    risk_level: 3
    params:
      - name: tokenIn
        type: address
        description: "Input token"
      - name: amountIn
        type: uint256
        description: "Amount to swap"
    execution:
      "eip155:*":
        type: evm_call
        to: { lit: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45" }
        abi:
          type: "function"
          name: "exactInputSingle"
          inputs:
            - { name: "tokenIn", type: "address" }
            - { name: "amountIn", type: "uint256" }
          outputs: []
        args:
          tokenIn: { ref: "params.tokenIn" }
          amountIn: { ref: "params.amountIn" }
`;

describe('ResolverContext', () => {
  let ctx: ResolverContext;

  beforeEach(() => {
    ctx = createContext();
  });

  it('creates empty context', () => {
    expect(ctx.protocols.size).toBe(0);
    expect(Object.keys(ctx.runtime.inputs)).toHaveLength(0);
    expect(Object.keys(ctx.runtime.query)).toHaveLength(0);
  });

  it('registers protocols', () => {
    const spec = parseProtocolSpec(SAMPLE_PROTOCOL);
    registerProtocol(ctx, spec);
    expect(ctx.protocols.has('uniswap-v3')).toBe(true);
  });
});

describe('parseProtocolRef', () => {
  it('parses protocol only', () => {
    const ref = parseProtocolRef('uniswap-v3');
    expect(ref.protocol).toBe('uniswap-v3');
    expect(ref.version).toBeUndefined();
  });

  it('parses protocol with version', () => {
    const ref = parseProtocolRef('uniswap-v3@1.0.0');
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
    const spec = resolveProtocolRef(ctx, 'uniswap-v3@0.0.2');
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
    const result = resolveAction(ctx, 'uniswap-v3/swap-exact-in');
    expect(result).not.toBeNull();
    expect(result?.actionId).toBe('swap-exact-in');
    expect(result?.action.description).toBe('Swap exact input amount');
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
    const result = resolveQuery(ctx, 'uniswap-v3/get-pool');
    expect(result).not.toBeNull();
    expect(result?.queryId).toBe('get-pool');
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
    setRef(ctx, 'inputs.amount', 1000);
    expect(resolveExpression('inputs.amount', ctx)).toBe(1000);
  });

  it('resolves node outputs', () => {
    setRef(ctx, 'nodes.get_pool.outputs.pool', '0xabc');
    setRef(ctx, 'nodes.get_pool.outputs.fee', 3000);
    expect(resolveExpression('nodes.get_pool.outputs.pool', ctx)).toBe('0xabc');
    expect(resolveExpression('nodes.get_pool.outputs.fee', ctx)).toBe(3000);
  });

  it('resolves ctx variables', () => {
    setRef(ctx, 'ctx.chain', 'eip155:1');
    setRef(ctx, 'ctx.sender', '0xuser');
    expect(resolveExpression('ctx.chain', ctx)).toBe('eip155:1');
    expect(resolveExpression('ctx.sender', ctx)).toBe('0xuser');
  });

  it('resolves query results', () => {
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
    setRef(ctx, 'inputs.amount', 1000);
    setRef(ctx, 'inputs.token', '0xWETH');
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
