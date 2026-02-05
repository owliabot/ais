import { describe, it, expect } from 'vitest';
import { protocol, pack, workflow, param, output } from '../src/index.js';

describe('ProtocolBuilder', () => {
  const testExecution = {
    'eip155:*': {
      type: 'evm_call' as const,
      contract: 'router',
      function: 'test',
      abi: '()',
      mapping: {},
    },
  };

  it('builds a minimal protocol', () => {
    const spec = protocol('test-protocol', '1.0.0')
      .deployment('eip155:1', { router: '0x1234567890123456789012345678901234567890' })
      .action('test', {
        description: 'Test action',
        risk_level: 2,
        execution: testExecution,
      })
      .build();

    expect(spec.schema).toBe('ais/1.0');
    expect(spec.meta.protocol).toBe('test-protocol');
    expect(spec.meta.version).toBe('1.0.0');
    expect(spec.deployments).toHaveLength(1);
    expect(spec.actions.test.description).toBe('Test action');
  });

  it('builds a full protocol', () => {
    const swapExecution = {
      'eip155:*': {
        type: 'evm_call' as const,
        contract: 'router',
        function: 'exactInputSingle',
        abi: '(address,address,uint256,uint256)',
        mapping: {
          tokenIn: 'params.tokenIn',
          tokenOut: 'params.tokenOut',
          amountIn: 'params.amountIn',
          amountOutMin: 'params.amountOutMin',
        },
      },
    };

    const queryExecution = {
      'eip155:*': {
        type: 'evm_read' as const,
        contract: 'quoter',
        function: 'quoteExactInputSingle',
        abi: '(address,address,uint256)',
        mapping: {
          tokenIn: 'params.tokenIn',
          tokenOut: 'params.tokenOut',
          amountIn: 'params.amountIn',
        },
      },
    };

    const spec = protocol('uniswap-v3', '1.0.0')
      .name('Uniswap V3')
      .description('Decentralized exchange protocol')
      .homepage('https://uniswap.org')
      .maintainer('uniswap.eth')
      .tags('dex', 'swap', 'amm')
      .deployment('eip155:1', {
        router: '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45',
        quoter: '0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6',
      })
      .deployment('eip155:8453', {
        router: '0x2626664c2603336E57B271c5C0b26F421741e481',
      })
      .query('quote-exact-in', {
        description: 'Quote exact input swap',
        execution: queryExecution,
        params: [
          param('tokenIn', 'address', { description: 'Input token' }),
          param('tokenOut', 'address', { description: 'Output token' }),
          param('amountIn', 'uint256', { description: 'Input amount' }),
        ],
        returns: [output('amountOut', 'uint256', { description: 'Output amount' })],
      })
      .action('swap-exact-in', {
        description: 'Swap exact input for maximum output',
        risk_level: 3,
        execution: swapExecution,
        params: [
          param('tokenIn', 'address', { description: 'Input token' }),
          param('tokenOut', 'address', { description: 'Output token' }),
          param('amountIn', 'uint256', { description: 'Input amount' }),
          param('amountOutMin', 'uint256', { description: 'Min output' }),
        ],
        requires_queries: ['quote-exact-in'],
      })
      .capabilities('evm_call', 'evm_read')
      .build();

    expect(spec.meta.name).toBe('Uniswap V3');
    expect(spec.meta.description).toBe('Decentralized exchange protocol');
    expect(spec.meta.tags).toEqual(['dex', 'swap', 'amm']);
    expect(spec.deployments).toHaveLength(2);
    expect(spec.queries?.['quote-exact-in'].params).toHaveLength(3);
    expect(spec.actions['swap-exact-in'].requires_queries).toContain('quote-exact-in');
    expect(spec.capabilities_required).toEqual(['evm_call', 'evm_read']);
  });

  it('converts to YAML', () => {
    const yaml = protocol('test', '1.0.0')
      .deployment('eip155:1', { router: '0x1234567890123456789012345678901234567890' })
      .action('test', {
        description: 'Test',
        risk_level: 1,
        execution: testExecution,
      })
      .toYAML();

    expect(yaml).toContain('schema: ais/1.0');
    expect(yaml).toContain('protocol: test');
  });

  it('converts to JSON', () => {
    const json = protocol('test', '1.0.0')
      .deployment('eip155:1', { router: '0x1234567890123456789012345678901234567890' })
      .action('test', {
        description: 'Test',
        risk_level: 1,
        execution: testExecution,
      })
      .toJSON();

    const parsed = JSON.parse(json);
    expect(parsed.schema).toBe('ais/1.0');
  });
});

describe('PackBuilder', () => {
  it('builds a minimal pack', () => {
    const p = pack('test-pack', '1.0.0')
      .include('protocol-a', '1.0.0')
      .build();

    expect(p.schema).toBe('ais-pack/1.0');
    expect(p.name).toBe('test-pack');
    expect(p.includes).toHaveLength(1);
    expect(p.includes[0].protocol).toBe('protocol-a');
  });

  it('builds a full pack', () => {
    const p = pack('safe-defi', '1.0.0')
      .description('Safe DeFi operations pack')
      .include('uniswap-v3', '1.0.0')
      .include('aave-v3', '1.0.0')
      .include('erc20', '1.0.0')
      .approvals({
        auto_execute_max_risk_level: 2,
        require_approval_min_risk_level: 3,
      })
      .constraints({
        max_slippage_bps: 100,
        allow_unlimited_approval: false,
      })
      .allowToken({
        chain: 'eip155:1',
        symbol: 'WETH',
        address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      })
      .allowToken({
        chain: 'eip155:1',
        symbol: 'USDC',
        address: '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48',
      })
      .tokenResolution({
        allow_symbol_input: true,
        require_allowlist_for_symbol_resolution: true,
      })
      .quoteProvider('oneinch')
      .quoteProvider('paraswap')
      .build();

    expect(p.description).toBe('Safe DeFi operations pack');
    expect(p.includes).toHaveLength(3);
    expect(p.policy?.hard_constraints_defaults?.max_slippage_bps).toBe(100);
    expect(p.token_policy?.allowlist).toHaveLength(2);
    expect(p.token_policy?.resolution?.allow_symbol_input).toBe(true);
    expect(p.providers?.quote?.enabled).toHaveLength(2);
  });

  it('supports shorthand constraint methods', () => {
    const p = pack('test', '1.0.0')
      .include('a', '1.0.0')
      .maxSlippage(50)
      .disallowUnlimitedApproval()
      .build();

    expect(p.policy?.hard_constraints_defaults?.max_slippage_bps).toBe(50);
    expect(p.policy?.hard_constraints_defaults?.allow_unlimited_approval).toBe(false);
  });
});

describe('WorkflowBuilder', () => {
  it('builds a minimal workflow', () => {
    const w = workflow('test-flow', '1.0.0')
      .node('step1', {
        type: 'action_ref',
        skill: 'erc20@1.0.0',
        action: 'transfer',
      })
      .build();

    expect(w.schema).toBe('ais-flow/1.0');
    expect(w.meta.name).toBe('test-flow');
    expect(w.nodes).toHaveLength(1);
  });

  it('builds a full workflow', () => {
    const w = workflow('swap-to-token', '1.0.0')
      .description('Swap ETH to target token')
      .requiresPack('safe-defi', '1.0.0')
      .requiredInput('target_token', 'address')
      .requiredInput('amount_in', 'uint256')
      .optionalInput('slippage_bps', 'uint256', 50)
      .query('get_quote', 'uniswap-v3@1.0.0', 'quote-exact-in', {
        args: {
          tokenIn: '${inputs.weth}',
          tokenOut: '${inputs.target_token}',
          amountIn: '${inputs.amount_in}',
        },
      })
      .action('approve', 'erc20@1.0.0', 'approve', {
        args: {
          spender: '${ctx.router}',
          amount: '${inputs.amount_in}',
        },
      })
      .action('swap', 'uniswap-v3@1.0.0', 'swap-exact-in', {
        args: {
          tokenOut: '${inputs.target_token}',
          amountIn: '${inputs.amount_in}',
          amountOutMin: '${nodes.get_quote.outputs.amountOut}',
        },
        requires: ['approve', 'get_quote'],
        condition: 'nodes.approve.outputs.success == true',
      })
      .output('amount_out', 'nodes.swap.outputs.amountOut')
      .build();

    expect(w.meta.description).toBe('Swap ETH to target token');
    expect(w.requires_pack).toEqual({ name: 'safe-defi', version: '1.0.0' });
    expect(Object.keys(w.inputs ?? {})).toHaveLength(3);
    expect(w.inputs?.slippage_bps.default).toBe(50);
    expect(w.nodes).toHaveLength(3);
    expect(w.nodes[2].requires_queries).toContain('approve');
    expect(w.outputs?.amount_out).toBe('nodes.swap.outputs.amountOut');
  });

  it('supports shorthand action/query methods', () => {
    const w = workflow('test', '1.0.0')
      .action('a1', 'proto@1.0.0', 'do-action')
      .query('q1', 'proto@1.0.0', 'get-data')
      .build();

    expect(w.nodes[0].type).toBe('action_ref');
    expect(w.nodes[0].action).toBe('do-action');
    expect(w.nodes[1].type).toBe('query_ref');
    expect(w.nodes[1].query).toBe('get-data');
  });
});

describe('param and output helpers', () => {
  it('creates param definitions', () => {
    const p1 = param('amount', 'uint256');
    expect(p1).toEqual({ name: 'amount', type: 'uint256' });

    const p2 = param('token', 'address', { description: 'Token address', required: true });
    expect(p2).toEqual({
      name: 'token',
      type: 'address',
      description: 'Token address',
      required: true,
    });
  });

  it('creates output definitions', () => {
    const o1 = output('result', 'uint256');
    expect(o1).toEqual({ name: 'result', type: 'uint256' });

    const o2 = output('pool', 'address', { description: 'Pool address' });
    expect(o2).toEqual({ name: 'pool', type: 'address', description: 'Pool address' });
  });
});

describe('Builder chaining', () => {
  const testExecution = {
    'eip155:*': {
      type: 'evm_call' as const,
      contract: 'r',
      function: 'm',
      abi: '()',
      mapping: {},
    },
  };

  const testQueryExecution = {
    'eip155:*': {
      type: 'evm_read' as const,
      contract: 'r',
      function: 'm',
      abi: '()',
      mapping: {},
    },
  };

  it('supports method chaining', () => {
    const spec = protocol('chain-test', '1.0.0')
      .description('Test')
      .deployment('eip155:1', { r: '0x1234567890123456789012345678901234567890' })
      .action('a', { description: 'A', risk_level: 1, execution: testExecution })
      .action('b', { description: 'B', risk_level: 1, execution: testExecution })
      .query('q', { description: 'Q', execution: testQueryExecution })
      .build();

    expect(Object.keys(spec.actions)).toHaveLength(2);
    expect(Object.keys(spec.queries ?? {})).toHaveLength(1);
  });

  it('is immutable-like (each method returns this)', () => {
    const builder = protocol('test', '1.0.0');
    const result = builder.description('desc');
    expect(result).toBe(builder);
  });
});
