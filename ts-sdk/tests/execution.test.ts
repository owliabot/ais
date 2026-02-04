import { describe, it, expect, beforeEach } from 'vitest';
import {
  buildTransaction,
  buildQuery,
  buildWorkflowTransactions,
  encodeFunctionSelector,
  encodeFunctionCall,
  encodeValue,
  buildFunctionSignature,
  keccak256,
  createContext,
  registerProtocol,
  setVariable,
  parseProtocolSpec,
  type ResolverContext,
} from '../src/index.js';

describe('keccak256', () => {
  it('hashes empty string correctly', () => {
    // keccak256("") = 0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
    const hash = keccak256('');
    expect(hash).toBe('0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470');
  });

  it('hashes "hello" correctly', () => {
    // keccak256("hello") = 0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8
    const hash = keccak256('hello');
    expect(hash).toBe('0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8');
  });
});

describe('encodeFunctionSelector', () => {
  it('encodes transfer(address,uint256)', () => {
    // transfer(address,uint256) = 0xa9059cbb
    const selector = encodeFunctionSelector('transfer(address,uint256)');
    expect(selector).toBe('0xa9059cbb');
  });

  it('encodes approve(address,uint256)', () => {
    // approve(address,uint256) = 0x095ea7b3
    const selector = encodeFunctionSelector('approve(address,uint256)');
    expect(selector).toBe('0x095ea7b3');
  });

  it('encodes balanceOf(address)', () => {
    // balanceOf(address) = 0x70a08231
    const selector = encodeFunctionSelector('balanceOf(address)');
    expect(selector).toBe('0x70a08231');
  });
});

describe('encodeValue', () => {
  it('encodes address', () => {
    const encoded = encodeValue('address', '0x1234567890123456789012345678901234567890');
    expect(encoded).toBe('0000000000000000000000001234567890123456789012345678901234567890');
  });

  it('encodes uint256', () => {
    const encoded = encodeValue('uint256', 1000n);
    expect(encoded).toBe('00000000000000000000000000000000000000000000000000000000000003e8');
  });

  it('encodes bool true', () => {
    const encoded = encodeValue('bool', true);
    expect(encoded).toBe('0000000000000000000000000000000000000000000000000000000000000001');
  });

  it('encodes bool false', () => {
    const encoded = encodeValue('bool', false);
    expect(encoded).toBe('0000000000000000000000000000000000000000000000000000000000000000');
  });
});

describe('buildFunctionSignature', () => {
  it('builds simple signature', () => {
    expect(buildFunctionSignature('transfer', ['address', 'uint256']))
      .toBe('transfer(address,uint256)');
  });

  it('builds signature with no params', () => {
    expect(buildFunctionSignature('pause', []))
      .toBe('pause()');
  });
});

describe('encodeFunctionCall', () => {
  it('encodes transfer call', () => {
    const data = encodeFunctionCall(
      'transfer(address,uint256)',
      ['address', 'uint256'],
      ['0x1234567890123456789012345678901234567890', 1000n]
    );
    
    // selector + address + amount
    expect(data.startsWith('0xa9059cbb')).toBe(true);
    expect(data.length).toBe(10 + 64 + 64); // selector(10) + 2 params(64 each)
  });

  it('encodes approve call', () => {
    const data = encodeFunctionCall(
      'approve(address,uint256)',
      ['address', 'uint256'],
      ['0x1234567890123456789012345678901234567890', BigInt('115792089237316195423570985008687907853269984665640564039457584007913129639935')]
    );
    
    expect(data.startsWith('0x095ea7b3')).toBe(true);
  });
});

const SAMPLE_PROTOCOL = `
schema: "ais/1.0"
meta:
  protocol: test-dex
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
      factory: "0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f"
  - chain: "eip155:8453"
    contracts:
      router: "0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24"
queries:
  get_reserves:
    contract: factory
    method: getReserves
    params:
      - name: pair
        type: address
    outputs:
      - name: reserve0
        type: uint112
      - name: reserve1
        type: uint112
actions:
  swap:
    contract: router
    method: swapExactTokensForTokens
    params:
      - name: amountIn
        type: uint256
      - name: amountOutMin
        type: uint256
      - name: path
        type: address[]
      - name: to
        type: address
      - name: deadline
        type: uint256
  approve:
    contract: router
    method: approve
    params:
      - name: spender
        type: address
      - name: amount
        type: uint256
        default: "115792089237316195423570985008687907853269984665640564039457584007913129639935"
`;

describe('buildTransaction', () => {
  let ctx: ResolverContext;
  let protocol: ReturnType<typeof parseProtocolSpec>;

  beforeEach(() => {
    ctx = createContext();
    protocol = parseProtocolSpec(SAMPLE_PROTOCOL);
    registerProtocol(ctx, protocol);
  });

  it('builds swap transaction', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions.swap,
      {
        amountIn: 1000000000000000000n,
        amountOutMin: 990000000000000000n,
        path: ['0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48'],
        to: '0x1234567890123456789012345678901234567890',
        deadline: 1700000000n,
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transaction.to).toBe('0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D');
      expect(result.transaction.chainId).toBe(1);
      expect(result.transaction.data.startsWith('0x')).toBe(true);
      expect(result.transaction.value).toBe(0n);
    }
  });

  it('builds transaction with default value', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions.approve,
      {
        spender: '0x1234567890123456789012345678901234567890',
        // amount not provided, should use default
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.resolvedParams.amount).toBe(
        BigInt('115792089237316195423570985008687907853269984665640564039457584007913129639935')
      );
    }
  });

  it('resolves expression parameters', () => {
    setVariable(ctx, 'inputs.recipient', '0xabcdef0123456789012345678901234567890abc');
    setVariable(ctx, 'inputs.amount', '2000000000000000000');

    const result = buildTransaction(
      protocol,
      protocol.actions.approve,
      {
        spender: '${inputs.recipient}',
        amount: '${inputs.amount}',
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.resolvedParams.spender).toBe('0xabcdef0123456789012345678901234567890abc');
      expect(result.resolvedParams.amount).toBe(2000000000000000000n);
    }
  });

  it('fails for unknown chain', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions.swap,
      { amountIn: 1n, amountOutMin: 1n, path: [], to: '0x0000000000000000000000000000000000000000', deadline: 1n },
      ctx,
      { chain: 'eip155:999' }
    );

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.error).toContain('not found');
    }
  });

  it('allows contract address override', () => {
    const customAddress = '0xcustom00000000000000000000000000000000';
    const result = buildTransaction(
      protocol,
      protocol.actions.approve,
      { spender: '0x0000000000000000000000000000000000000000' },
      ctx,
      { chain: 'eip155:1', contractAddress: customAddress }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transaction.to).toBe(customAddress);
    }
  });
});

describe('buildQuery', () => {
  let ctx: ResolverContext;
  let protocol: ReturnType<typeof parseProtocolSpec>;

  beforeEach(() => {
    ctx = createContext();
    protocol = parseProtocolSpec(SAMPLE_PROTOCOL);
    registerProtocol(ctx, protocol);
  });

  it('builds query call', () => {
    const result = buildQuery(
      protocol,
      protocol.queries!.get_reserves,
      { pair: '0x1234567890123456789012345678901234567890' },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transaction.to).toBe('0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f');
      expect(result.transaction.value).toBe(0n);
    }
  });
});

describe('buildWorkflowTransactions', () => {
  let ctx: ResolverContext;
  let protocols: Map<string, ReturnType<typeof parseProtocolSpec>>;

  beforeEach(() => {
    ctx = createContext();
    const protocol = parseProtocolSpec(SAMPLE_PROTOCOL);
    registerProtocol(ctx, protocol);
    protocols = ctx.protocols;
  });

  it('builds multiple transactions from workflow nodes', () => {
    const nodes = [
      {
        skill: 'test-dex@1.0.0',
        action: 'approve',
        args: {
          spender: '0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D',
          amount: 1000000000000000000n,
        },
      },
      {
        skill: 'test-dex@1.0.0',
        query: 'get_reserves',
        args: {
          pair: '0x1234567890123456789012345678901234567890',
        },
      },
    ];

    const results = buildWorkflowTransactions(protocols, nodes, ctx, 'eip155:1');

    expect(results).toHaveLength(2);
    expect(results[0].success).toBe(true);
    expect(results[1].success).toBe(true);
  });
});
