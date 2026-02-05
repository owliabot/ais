/**
 * Tests for pre-authorize module
 */
import { describe, it, expect } from 'vitest';
import {
  buildPreAuthorize,
  buildAllowanceCheckData,
  getPreAuthorizeQueries,
  PERMIT2_ADDRESS,
  MAX_UINT256,
  type PreAuthorizeConfig,
  type PreAuthorizeContext,
} from '../../src/execution/pre-authorize.js';
import { createContext, type ResolverContext } from '../../src/resolver/index.js';
import { Evaluator, type CELContext } from '../../src/cel/evaluator.js';
import type { ProtocolSpec } from '../../src/schema/index.js';

// Test fixtures
const MOCK_TOKEN = '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48'; // USDC
const MOCK_ROUTER = '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45'; // SwapRouter02
const MOCK_WALLET = '0x1234567890123456789012345678901234567890';

const mockProtocol: ProtocolSpec = {
  ais_version: '1.0',
  protocol: 'test-protocol',
  version: '1.0.0',
  description: 'Test protocol',
  deployments: [
    {
      chain: 'eip155:1',
      contracts: {
        router: MOCK_ROUTER,
        usdc: MOCK_TOKEN,
      },
    },
  ],
  actions: {},
};

function createTestContext(): { ctx: ResolverContext; celCtx: CELContext; evaluator: Evaluator } {
  const ctx = createContext();
  ctx.variables['params.token_in.address'] = MOCK_TOKEN;
  ctx.variables['params.amount'] = 1000000; // 1 USDC
  ctx.variables['calculated.amount_atomic'] = 1000000n;
  ctx.variables['ctx.wallet_address'] = MOCK_WALLET;

  const celCtx: CELContext = {
    params: {
      token_in: { address: MOCK_TOKEN },
      amount: 1000000,
    },
    calculated: {
      amount_atomic: 1000000,
    },
    ctx: {
      wallet_address: MOCK_WALLET,
    },
    contracts: {
      router: MOCK_ROUTER,
    },
  };

  const evaluator = new Evaluator();

  return { ctx, celCtx, evaluator };
}

describe('buildAllowanceCheckData', () => {
  it('should build correct allowance check calldata', () => {
    const result = buildAllowanceCheckData(MOCK_TOKEN, MOCK_WALLET, MOCK_ROUTER);

    expect(result.to).toBe(MOCK_TOKEN);
    expect(result.data).toMatch(/^0x/);
    // allowance(address,address) selector = 0xdd62ed3e
    expect(result.data.slice(0, 10)).toBe('0xdd62ed3e');
  });
});

describe('getPreAuthorizeQueries', () => {
  it('should return allowance query for approve method', () => {
    const { ctx, celCtx, evaluator } = createTestContext();
    const config: PreAuthorizeConfig = {
      method: 'approve',
      token: 'params.token_in.address',
      spender: 'router',
      amount: 'calculated.amount_atomic',
    };

    const queries = getPreAuthorizeQueries(
      config,
      ctx,
      celCtx,
      evaluator,
      mockProtocol,
      'eip155:1',
      MOCK_WALLET
    );

    expect(queries).toHaveLength(1);
    expect(queries[0].id).toBe('pre_auth_allowance');
    expect(queries[0].to).toBe(MOCK_TOKEN);
  });

  it('should return both allowance queries for permit2 method', () => {
    const { ctx, celCtx, evaluator } = createTestContext();
    const config: PreAuthorizeConfig = {
      method: 'permit2',
      token: 'params.token_in.address',
      spender: 'router',
      amount: 'calculated.amount_atomic',
    };

    const queries = getPreAuthorizeQueries(
      config,
      ctx,
      celCtx,
      evaluator,
      mockProtocol,
      'eip155:1',
      MOCK_WALLET
    );

    expect(queries).toHaveLength(2);
    expect(queries[0].id).toBe('pre_auth_allowance');
    expect(queries[1].id).toBe('pre_auth_permit2_allowance');
  });
});

describe('buildPreAuthorize', () => {
  describe('approve method', () => {
    it('should skip if allowance sufficient', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: 'params.token_in.address',
        spender: 'router',
        amount: 'calculated.amount_atomic',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 2000000n, // More than needed
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(false);
      expect(result.approveTx).toBeUndefined();
    });

    it('should build approve tx if allowance insufficient', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: 'params.token_in.address',
        spender: 'router',
        amount: 'calculated.amount_atomic',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 0n,
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      expect(result.approveTx).toBeDefined();
      expect(result.approveTx!.to).toBe(MOCK_TOKEN);
      expect(result.approveTx!.chainId).toBe(1);
      expect(result.approveTx!.stepId).toBe('pre_authorize_approve');
    });

    it('should resolve direct token address', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: MOCK_TOKEN,
        spender: 'router',
        amount: '1000000',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 0n,
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      expect(result.approveTx!.to).toBe(MOCK_TOKEN);
    });

    it('should handle MAX_UINT256 amount', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: MOCK_TOKEN,
        spender: 'router',
        amount: 'MAX_UINT256',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 0n,
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      expect(result.approveTx).toBeDefined();
    });
  });

  describe('permit method', () => {
    it('should build permit data for signing', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'permit',
        token: 'params.token_in.address',
        spender: 'router',
        amount: 'calculated.amount_atomic',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 0n,
        tokenName: 'USD Coin',
        nonce: 0n,
        deadline: BigInt(Math.floor(Date.now() / 1000) + 3600),
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      expect(result.permitData).toBeDefined();
      expect(result.permitData!.domain.name).toBe('USD Coin');
      expect(result.permitData!.domain.chainId).toBe(1);
      expect(result.permitData!.domain.verifyingContract).toBe(MOCK_TOKEN);
      expect(result.permitData!.primaryType).toBe('Permit');
      expect(result.permitData!.message.owner).toBe(MOCK_WALLET);
      expect(result.permitData!.message.spender).toBe(MOCK_ROUTER);
    });

    it('should throw if token name not provided', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'permit',
        token: MOCK_TOKEN,
        spender: 'router',
        amount: '1000000',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        currentAllowance: 0n,
        // tokenName missing
      };

      expect(() =>
        buildPreAuthorize(
          config,
          ctx,
          celCtx,
          evaluator,
          mockProtocol,
          'eip155:1',
          preAuthCtx
        )
      ).toThrow('Token name required');
    });
  });

  describe('permit2 method', () => {
    it('should build permit2 data with approval tx if needed', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'permit2',
        token: 'params.token_in.address',
        spender: 'router',
        amount: 'calculated.amount_atomic',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        permit2Allowance: 0n, // Need to approve Permit2 first
        nonce: 0n,
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      // Should have Permit2 approval tx
      expect(result.permit2ApproveTx).toBeDefined();
      expect(result.permit2ApproveTx!.to).toBe(MOCK_TOKEN);
      expect(result.permit2ApproveTx!.stepId).toBe('pre_authorize_permit2_approve');
      
      // Should have permit data
      expect(result.permitData).toBeDefined();
      expect(result.permitData!.domain.name).toBe('Permit2');
      expect(result.permitData!.domain.verifyingContract).toBe(PERMIT2_ADDRESS);
      expect(result.permitData!.primaryType).toBe('PermitSingle');
    });

    it('should skip permit2 approval if already approved', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'permit2',
        token: 'params.token_in.address',
        spender: 'router',
        amount: 'calculated.amount_atomic',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
        permit2Allowance: MAX_UINT256, // Already approved
        nonce: 0n,
      };

      const result = buildPreAuthorize(
        config,
        ctx,
        celCtx,
        evaluator,
        mockProtocol,
        'eip155:1',
        preAuthCtx
      );

      expect(result.needed).toBe(true);
      // Should NOT have Permit2 approval tx
      expect(result.permit2ApproveTx).toBeUndefined();
      // Should still have permit data for signing
      expect(result.permitData).toBeDefined();
    });
  });

  describe('error handling', () => {
    it('should throw for invalid token reference', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: 'invalid.ref',
        spender: 'router',
        amount: '1000000',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
      };

      expect(() =>
        buildPreAuthorize(
          config,
          ctx,
          celCtx,
          evaluator,
          mockProtocol,
          'eip155:1',
          preAuthCtx
        )
      ).toThrow('Cannot resolve token address');
    });

    it('should throw for invalid spender', () => {
      const { ctx, celCtx, evaluator } = createTestContext();
      const config: PreAuthorizeConfig = {
        method: 'approve',
        token: MOCK_TOKEN,
        spender: 'nonexistent',
        amount: '1000000',
      };

      const preAuthCtx: PreAuthorizeContext = {
        walletAddress: MOCK_WALLET,
      };

      expect(() =>
        buildPreAuthorize(
          config,
          ctx,
          celCtx,
          evaluator,
          mockProtocol,
          'eip155:1',
          preAuthCtx
        )
      ).toThrow('Spender contract "nonexistent" not found');
    });
  });
});

describe('constants', () => {
  it('should export correct Permit2 address', () => {
    expect(PERMIT2_ADDRESS).toBe('0x000000000022D473030F116dDEE9F6B43aC78BA3');
  });

  it('should export correct MAX_UINT256', () => {
    expect(MAX_UINT256).toBe(
      BigInt('0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff')
    );
  });
});
