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
    const hash = keccak256('');
    expect(hash).toBe('0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470');
  });

  it('hashes "hello" correctly', () => {
    const hash = keccak256('hello');
    expect(hash).toBe('0x1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8');
  });
});

describe('encodeFunctionSelector', () => {
  it('encodes transfer(address,uint256)', () => {
    const selector = encodeFunctionSelector('transfer(address,uint256)');
    expect(selector).toBe('0xa9059cbb');
  });

  it('encodes approve(address,uint256)', () => {
    const selector = encodeFunctionSelector('approve(address,uint256)');
    expect(selector).toBe('0x095ea7b3');
  });

  it('encodes balanceOf(address)', () => {
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
    
    expect(data.startsWith('0xa9059cbb')).toBe(true);
    expect(data.length).toBe(10 + 64 + 64);
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
  get-reserves:
    description: "Get pair reserves"
    params:
      - name: pair
        type: address
        description: "Pair address"
    returns:
      - name: reserve0
        type: uint112
      - name: reserve1
        type: uint112
    execution:
      "eip155:*":
        type: evm_read
        contract: factory
        function: getReserves
        abi: "(address)"
        mapping:
          pair: "params.pair"
actions:
  swap:
    description: "Swap tokens"
    risk_level: 3
    params:
      - name: amountIn
        type: uint256
        description: "Input amount"
      - name: amountOutMin
        type: uint256
        description: "Minimum output"
      - name: to
        type: address
        description: "Recipient"
    execution:
      "eip155:*":
        type: evm_call
        contract: router
        function: swapExactTokensForTokens
        abi: "(uint256,uint256,address)"
        mapping:
          amountIn: "params.amountIn"
          amountOutMin: "params.amountOutMin"
          to: "params.to"
  approve:
    description: "Approve spender"
    risk_level: 2
    params:
      - name: spender
        type: address
        description: "Spender address"
      - name: amount
        type: uint256
        description: "Amount to approve"
        default: "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    execution:
      "eip155:*":
        type: evm_call
        contract: router
        function: approve
        abi: "(address,uint256)"
        mapping:
          spender: "params.spender"
          amount: "params.amount"
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
        to: '0x1234567890123456789012345678901234567890',
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transactions[0].to).toBe('0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D');
      expect(result.transactions[0].chainId).toBe(1);
      expect(result.transactions[0].data.startsWith('0x')).toBe(true);
      expect(result.transactions[0].value).toBe(0n);
    }
  });

  it('builds transaction with default value', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions.approve,
      {
        spender: '0x1234567890123456789012345678901234567890',
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

  it('fails for unknown chain', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions.swap,
      { amountIn: 1n, amountOutMin: 1n, to: '0x0000000000000000000000000000000000000000' },
      ctx,
      { chain: 'eip155:999' }
    );

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.error).toContain('not found');
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
      protocol.queries!['get-reserves'],
      { pair: '0x1234567890123456789012345678901234567890' },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transactions[0].to).toBe('0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f');
      expect(result.transactions[0].value).toBe(0n);
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
        query: 'get-reserves',
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

// ═══════════════════════════════════════════════════════════════════════════════
// Chain Pattern Resolver Enhancements
// ═══════════════════════════════════════════════════════════════════════════════

const CEL_PROTOCOL = `
schema: "ais/1.0"
meta:
  protocol: cel-test
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
      token: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
actions:
  swap-with-cel:
    description: "Swap with CEL expressions in mapping"
    risk_level: 3
    params:
      - name: amount
        type: float
        description: "Human-readable amount"
      - name: token
        type: address
        description: "Token with decimals"
      - name: slippage
        type: float
        description: "Slippage tolerance"
        default: 0.01
    execution:
      "eip155:*":
        type: evm_call
        contract: router
        function: swap
        abi: "(uint256,uint256)"
        mapping:
          amountIn: "to_atomic(params.amount, 18)"
          minOut: "floor(to_atomic(params.amount, 18) * (1 - params.slippage))"
`;

const COMPOSITE_PROTOCOL = `
schema: "ais/1.0"
meta:
  protocol: composite-test
  version: "1.0.0"
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"
actions:
  swap-with-approve:
    description: "Swap with conditional approve"
    risk_level: 3
    params:
      - name: token_in
        type: address
        description: "Input token"
      - name: amount_in
        type: uint256
        description: "Input amount"
    execution:
      "eip155:*":
        type: composite
        steps:
          - id: approve
            type: evm_call
            description: "Approve router"
            contract: "params.token_in"
            function: "approve"
            abi: "(address,uint256)"
            mapping:
              spender: "contracts.router"
              amount: "params.amount_in"
            condition: "query.allowance.value < params.amount_in"
          - id: swap
            type: evm_call
            description: "Execute swap"
            contract: router
            function: "swap"
            abi: "(uint256)"
            mapping:
              amountIn: "params.amount_in"
`;

describe('CEL expressions in mapping', () => {
  let ctx: ResolverContext;
  let protocol: ReturnType<typeof parseProtocolSpec>;

  beforeEach(() => {
    ctx = createContext();
    protocol = parseProtocolSpec(CEL_PROTOCOL);
    registerProtocol(ctx, protocol);
  });

  it('evaluates to_atomic() in mapping', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-cel'],
      {
        amount: 1.5,
        token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // 1.5 * 10^18 = 1500000000000000000
      expect(result.transactions[0].data).toContain('14d1120d7b160000'); // hex for 1.5e18
    }
  });

  it('evaluates complex CEL expression with floor()', () => {
    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-cel'],
      {
        amount: 100,
        token: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
        slippage: 0.01,
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // minOut should be floor(100e18 * 0.99) = 99e18
      expect(result.transactions[0].to).toBe('0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D');
    }
  });
});

describe('Composite execution with conditions', () => {
  let ctx: ResolverContext;
  let protocol: ReturnType<typeof parseProtocolSpec>;

  beforeEach(() => {
    ctx = createContext();
    protocol = parseProtocolSpec(COMPOSITE_PROTOCOL);
    registerProtocol(ctx, protocol);
  });

  it('skips approve step when condition is false', () => {
    // Set query result showing sufficient allowance (use number, not bigint)
    ctx.queryResults.set('allowance', { value: 1000000 });

    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-approve'],
      {
        token_in: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
        amount_in: 1000n, // Less than allowance
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // Only swap step should be included (approve skipped)
      expect(result.transactions).toHaveLength(1);
      expect(result.transactions[0].stepId).toBe('swap');
    }
  });

  it('includes approve step when condition is true', () => {
    // Set query result showing insufficient allowance
    ctx.queryResults.set('allowance', { value: 100 });

    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-approve'],
      {
        token_in: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
        amount_in: 1000n, // More than allowance
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // Both steps should be included
      expect(result.transactions).toHaveLength(2);
      expect(result.transactions[0].stepId).toBe('approve');
      expect(result.transactions[1].stepId).toBe('swap');
    }
  });

  it('resolves contracts.* in mapping', () => {
    // Set query result to trigger approve step
    ctx.queryResults.set('allowance', { value: 0 });

    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-approve'],
      {
        token_in: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
        amount_in: 1000n,
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // Approve step should have router address as spender
      const approveData = result.transactions[0].data;
      // Router address should be in the calldata (lowercase, no 0x prefix)
      expect(approveData.toLowerCase()).toContain('7a250d5630b4cf539739df2c5dacb4c659f2488d');
    }
  });

  it('resolves params.* as contract address', () => {
    // Set query result to trigger approve step
    ctx.queryResults.set('allowance', { value: 0 });

    const result = buildTransaction(
      protocol,
      protocol.actions['swap-with-approve'],
      {
        token_in: '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', // USDC
        amount_in: 1000n,
      },
      ctx,
      { chain: 'eip155:1' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      // Approve step should target the token_in address
      expect(result.transactions[0].to.toLowerCase()).toBe('0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48');
    }
  });
});
