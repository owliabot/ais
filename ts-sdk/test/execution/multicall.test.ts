/**
 * Tests for evm_multicall execution
 */
import { describe, it, expect } from 'vitest';
import {
  buildEvmMulticall,
  buildMulticallCalls,
  encodeStandardMulticall,
  encodeMulticall3,
  encodeUniversalRouter,
  type EncodedCall,
} from '../../src/execution/multicall.js';
import { createContext, type ResolverContext } from '../../src/resolver/index.js';
import { Evaluator, type CELContext } from '../../src/cel/evaluator.js';
import type { ProtocolSpec, EvmMulticall } from '../../src/schema/index.js';

// Test fixtures
const MOCK_ROUTER = '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45';
const MOCK_TOKEN_A = '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48';
const MOCK_TOKEN_B = '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2';
const MOCK_WALLET = '0x1234567890123456789012345678901234567890';

const mockProtocol: ProtocolSpec = {
  ais_version: '1.0',
  protocol: 'test-multicall',
  version: '1.0.0',
  description: 'Test protocol',
  deployments: [
    {
      chain: 'eip155:1',
      contracts: {
        router: MOCK_ROUTER,
        tokenA: MOCK_TOKEN_A,
        tokenB: MOCK_TOKEN_B,
      },
    },
  ],
  actions: {},
};

const mockMulticallSpec: EvmMulticall = {
  type: 'evm_multicall',
  contract: 'router',
  calls: [
    {
      function: 'approve',
      abi: '(address,uint256)',
      mapping: {
        spender: 'contracts.router',
        amount: '1000000',
      },
    },
    {
      function: 'transfer',
      abi: '(address,uint256)',
      mapping: {
        to: 'ctx.wallet_address',
        amount: '500000',
      },
    },
  ],
  deadline: 'calculated.deadline',
};

function createTestContext(): { ctx: ResolverContext; celCtx: CELContext; evaluator: Evaluator } {
  const ctx = createContext();
  ctx.variables['ctx.wallet_address'] = MOCK_WALLET;
  ctx.variables['calculated.deadline'] = BigInt(Math.floor(Date.now() / 1000) + 3600);

  const celCtx: CELContext = {
    ctx: {
      wallet_address: MOCK_WALLET,
    },
    calculated: {
      deadline: Math.floor(Date.now() / 1000) + 3600,
    },
    contracts: {
      router: MOCK_ROUTER,
      tokenA: MOCK_TOKEN_A,
      tokenB: MOCK_TOKEN_B,
    },
  };

  const evaluator = new Evaluator();

  return { ctx, celCtx, evaluator };
}

describe('buildMulticallCalls', () => {
  it('should build individual call data for each call', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const calls = buildMulticallCalls(
      mockProtocol,
      mockMulticallSpec,
      ctx,
      celCtx,
      evaluator,
      'eip155:1'
    );

    expect(calls).toHaveLength(2);
    expect(calls[0].data).toMatch(/^0x/);
    expect(calls[1].data).toMatch(/^0x/);
    // approve(address,uint256) selector = 0x095ea7b3
    expect(calls[0].data.slice(0, 10)).toBe('0x095ea7b3');
    // transfer(address,uint256) selector = 0xa9059cbb
    expect(calls[1].data.slice(0, 10)).toBe('0xa9059cbb');
  });

  it('should skip calls with false conditions', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const specWithCondition: EvmMulticall = {
      type: 'evm_multicall',
      contract: 'router',
      calls: [
        {
          function: 'approve',
          abi: '(address,uint256)',
          mapping: { spender: MOCK_ROUTER, amount: '1000000' },
          condition: 'true',
        },
        {
          function: 'transfer',
          abi: '(address,uint256)',
          mapping: { to: MOCK_WALLET, amount: '500000' },
          condition: 'false', // Should be skipped
        },
      ],
    };

    const calls = buildMulticallCalls(
      mockProtocol,
      specWithCondition,
      ctx,
      celCtx,
      evaluator,
      'eip155:1'
    );

    expect(calls).toHaveLength(1);
    expect(calls[0].data.slice(0, 10)).toBe('0x095ea7b3');
  });
});

describe('buildEvmMulticall', () => {
  it('should build standard multicall transaction', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const tx = buildEvmMulticall(
      mockProtocol,
      mockMulticallSpec,
      ctx,
      celCtx,
      evaluator,
      'eip155:1',
      { chain: 'eip155:1', style: 'standard' }
    );

    expect(tx.to).toBe(MOCK_ROUTER);
    expect(tx.data).toMatch(/^0x/);
    expect(tx.chainId).toBe(1);
    expect(tx.value).toBe(0n);
    expect(tx.stepDescription).toContain('2 calls');
  });

  it('should build multicall3 transaction', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const tx = buildEvmMulticall(
      mockProtocol,
      mockMulticallSpec,
      ctx,
      celCtx,
      evaluator,
      'eip155:1',
      { chain: 'eip155:1', style: 'multicall3' }
    );

    expect(tx.to).toBe(MOCK_ROUTER);
    expect(tx.data).toMatch(/^0x/);
    expect(tx.chainId).toBe(1);
  });

  it('should build universal router transaction', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const tx = buildEvmMulticall(
      mockProtocol,
      mockMulticallSpec,
      ctx,
      celCtx,
      evaluator,
      'eip155:1',
      { chain: 'eip155:1', style: 'universal_router' }
    );

    expect(tx.to).toBe(MOCK_ROUTER);
    expect(tx.data).toMatch(/^0x/);
    expect(tx.chainId).toBe(1);
  });

  it('should throw if no calls after condition evaluation', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const specAllSkipped: EvmMulticall = {
      type: 'evm_multicall',
      contract: 'router',
      calls: [
        {
          function: 'approve',
          abi: '(address,uint256)',
          mapping: { spender: MOCK_ROUTER, amount: '1000000' },
          condition: 'false',
        },
      ],
    };

    expect(() =>
      buildEvmMulticall(
        mockProtocol,
        specAllSkipped,
        ctx,
        celCtx,
        evaluator,
        'eip155:1'
      )
    ).toThrow('No calls to execute');
  });

  it('should resolve contract from deployments', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const tx = buildEvmMulticall(
      mockProtocol,
      mockMulticallSpec,
      ctx,
      celCtx,
      evaluator,
      'eip155:1'
    );

    expect(tx.to).toBe(MOCK_ROUTER);
  });

  it('should use direct address if provided', () => {
    const { ctx, celCtx, evaluator } = createTestContext();

    const specWithAddress: EvmMulticall = {
      type: 'evm_multicall',
      contract: '0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef',
      calls: [
        {
          function: 'test',
          abi: '()',
          mapping: {},
        },
      ],
    };

    const tx = buildEvmMulticall(
      mockProtocol,
      specWithAddress,
      ctx,
      celCtx,
      evaluator,
      'eip155:1'
    );

    expect(tx.to).toBe('0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef');
  });
});

describe('encoding functions', () => {
  const mockCalls: EncodedCall[] = [
    { data: '0x095ea7b3' + '0'.repeat(128), target: MOCK_TOKEN_A },
    { data: '0xa9059cbb' + '0'.repeat(128), target: MOCK_TOKEN_B },
  ];

  describe('encodeStandardMulticall', () => {
    it('should encode as multicall(bytes[])', () => {
      const encoded = encodeStandardMulticall(mockCalls);

      expect(encoded).toMatch(/^0x/);
      // multicall(bytes[]) selector
      expect(encoded.slice(0, 10)).toBe('0xac9650d8');
    });
  });

  describe('encodeMulticall3', () => {
    it('should encode as aggregate3', () => {
      const encoded = encodeMulticall3(mockCalls);

      expect(encoded).toMatch(/^0x/);
      // aggregate3 selector = 0x82ad56cb
      expect(encoded.slice(0, 10)).toBe('0x82ad56cb');
    });
  });

  describe('encodeUniversalRouter', () => {
    it('should encode as execute(bytes,bytes[],uint256)', () => {
      const deadline = BigInt(Math.floor(Date.now() / 1000) + 3600);
      const callsWithCommands = mockCalls.map((c, i) => ({
        ...c,
        command: i,
      }));

      const encoded = encodeUniversalRouter(callsWithCommands, deadline);

      expect(encoded).toMatch(/^0x/);
      // execute(bytes,bytes[],uint256) selector = 0x3593564c
      expect(encoded.slice(0, 10)).toBe('0x3593564c');
    });
  });
});

describe('integration with buildTransaction', () => {
  it('should build multicall via buildTransaction', async () => {
    const { buildTransaction } = await import('../../src/execution/builder.js');
    const { ctx, celCtx, evaluator } = createTestContext();

    const action = {
      description: 'Test multicall action',
      risk_level: 2,
      execution: {
        'eip155:1': mockMulticallSpec,
      },
    };

    const result = buildTransaction(
      mockProtocol,
      action,
      {},
      ctx,
      { chain: 'eip155:1', multicallStyle: 'standard' }
    );

    expect(result.success).toBe(true);
    if (result.success) {
      expect(result.transactions).toHaveLength(1);
      expect(result.transactions[0].to).toBe(MOCK_ROUTER);
    }
  });
});
