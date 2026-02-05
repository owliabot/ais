/**
 * Integration tests - test full SDK flows with mock data
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import {
  loadDirectoryAsContext,
  loadWorkflow,
  loadPack,
  validateWorkflow,
  validateConstraints,
  resolveAction,
  resolveQuery,
  resolveExpressionString,
  setVariable,
  getWorkflowProtocols,
  getWorkflowDependencies,
  expandPack,
  getContractAddress,
  getSupportedChains,
  type ResolverContext,
  type Workflow,
  type Pack,
} from '../src/index.js';

const TEST_DIR = '/tmp/ais-sdk-integration-test';

const UNISWAP_PROTOCOL = `
schema: "ais/1.0"
meta:
  protocol: uniswap-v3
  version: "1.0.0"
  name: Uniswap V3
  description: DEX protocol for token swaps
deployments:
  - chain: "eip155:1"
    contracts:
      router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
      quoter: "0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6"
      factory: "0x1F98431c8aD98523631AE4a59f267346ea31F984"
  - chain: "eip155:8453"
    contracts:
      router: "0x2626664c2603336E57B271c5C0b26F421741e481"
      quoter: "0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a"
queries:
  get-pool:
    description: "Get pool address for token pair"
    params:
      - name: tokenA
        type: address
        description: "First token"
      - name: tokenB
        type: address
        description: "Second token"
      - name: fee
        type: uint24
        description: "Fee tier"
    returns:
      - name: pool
        type: address
    execution:
      "eip155:*":
        type: evm_read
        contract: factory
        function: getPool
        abi: "(address,address,uint24)"
        mapping:
          tokenA: "params.tokenA"
          tokenB: "params.tokenB"
          fee: "params.fee"
  quote-exact-in:
    description: "Get quote for exact input swap"
    params:
      - name: tokenIn
        type: address
        description: "Input token"
      - name: tokenOut
        type: address
        description: "Output token"
      - name: amountIn
        type: uint256
        description: "Input amount"
    returns:
      - name: amountOut
        type: uint256
    execution:
      "eip155:*":
        type: evm_read
        contract: quoter
        function: quoteExactInputSingle
        abi: "(address,address,uint256)"
        mapping:
          tokenIn: "params.tokenIn"
          tokenOut: "params.tokenOut"
          amountIn: "params.amountIn"
actions:
  swap-exact-in:
    description: "Swap exact input for maximum output"
    risk_level: 3
    params:
      - name: tokenIn
        type: address
        description: "Input token"
      - name: tokenOut
        type: address
        description: "Output token"
      - name: fee
        type: uint24
        description: "Fee tier"
      - name: amountIn
        type: uint256
        description: "Input amount"
      - name: amountOutMin
        type: uint256
        description: "Minimum output"
    requires_queries:
      - quote-exact-in
    execution:
      "eip155:*":
        type: evm_call
        contract: router
        function: exactInputSingle
        abi: "(address,address,uint24,uint256,uint256)"
        mapping:
          tokenIn: "params.tokenIn"
          tokenOut: "params.tokenOut"
          fee: "params.fee"
          amountIn: "params.amountIn"
          amountOutMin: "params.amountOutMin"
`;

const ERC20_PROTOCOL = `
schema: "ais/1.0"
meta:
  protocol: erc20
  version: "1.0.0"
  name: ERC20 Token Standard
deployments:
  - chain: "eip155:1"
    contracts: {}
  - chain: "eip155:8453"
    contracts: {}
queries:
  allowance:
    description: "Check token allowance"
    params:
      - name: owner
        type: address
        description: "Token owner"
      - name: spender
        type: address
        description: "Approved spender"
    returns:
      - name: amount
        type: uint256
    execution:
      "eip155:*":
        type: evm_read
        contract: token
        function: allowance
        abi: "(address,address)"
        mapping:
          owner: "params.owner"
          spender: "params.spender"
  balance:
    description: "Check token balance"
    params:
      - name: account
        type: address
        description: "Account to check"
    returns:
      - name: balance
        type: uint256
    execution:
      "eip155:*":
        type: evm_read
        contract: token
        function: balanceOf
        abi: "(address)"
        mapping:
          account: "params.account"
actions:
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
    execution:
      "eip155:*":
        type: evm_call
        contract: token
        function: approve
        abi: "(address,uint256)"
        mapping:
          spender: "params.spender"
          amount: "params.amount"
  transfer:
    description: "Transfer tokens"
    risk_level: 2
    params:
      - name: to
        type: address
        description: "Recipient"
      - name: amount
        type: uint256
        description: "Amount"
    execution:
      "eip155:*":
        type: evm_call
        contract: token
        function: transfer
        abi: "(address,uint256)"
        mapping:
          to: "params.to"
          amount: "params.amount"
`;

const TEST_PACK = `
schema: "ais-pack/1.0"
name: safe-defi-pack
version: "1.0.0"
description: Safe DeFi operations with conservative constraints
includes:
  - protocol: uniswap-v3
    version: "1.0.0"
  - protocol: erc20
    version: "1.0.0"
policy:
  risk_threshold: 3
  approval_required:
    - flash_loan
    - unlimited_approval
  hard_constraints:
    max_slippage_bps: 100
    allow_unlimited_approval: false
token_policy:
  allowlist:
    - chain: "eip155:1"
      symbol: WETH
      address: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
    - chain: "eip155:1"
      symbol: USDC
      address: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
  resolution:
    require_allowlist_for_symbol_resolution: true
`;

const TEST_WORKFLOW = `
schema: "ais-flow/1.0"
meta:
  name: swap-to-token
  version: "1.0.0"
  description: Swap ETH to target token
inputs:
  target_token:
    type: address
    required: true
  amount_in:
    type: uint256
    required: true
  slippage_bps:
    type: uint256
    default: 50
nodes:
  - id: check_allowance
    type: query_ref
    skill: "erc20@1.0.0"
    query: allowance
    args:
      owner: "\${ctx.sender}"
      spender: "\${ctx.router}"
  - id: approve_if_needed
    type: action_ref
    skill: "erc20@1.0.0"
    action: approve
    args:
      spender: "\${ctx.router}"
      amount: "\${inputs.amount_in}"
    condition: "nodes.check_allowance.outputs.amount < inputs.amount_in"
  - id: get_quote
    type: query_ref
    skill: "uniswap-v3@1.0.0"
    query: quote-exact-in
    args:
      tokenIn: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
      tokenOut: "\${inputs.target_token}"
      amountIn: "\${inputs.amount_in}"
    requires_queries:
      - approve_if_needed
  - id: swap
    type: action_ref
    skill: "uniswap-v3@1.0.0"
    action: swap-exact-in
    args:
      tokenIn: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
      tokenOut: "\${inputs.target_token}"
      amountIn: "\${inputs.amount_in}"
      amountOutMin: "\${nodes.get_quote.outputs.amountOut}"
    requires_queries:
      - get_quote
outputs:
  amount_out: "nodes.swap.outputs.amountOut"
`;

beforeAll(async () => {
  await mkdir(TEST_DIR, { recursive: true });
  await writeFile(join(TEST_DIR, 'uniswap-v3.ais.yaml'), UNISWAP_PROTOCOL);
  await writeFile(join(TEST_DIR, 'erc20.ais.yaml'), ERC20_PROTOCOL);
  await writeFile(join(TEST_DIR, 'safe-defi.ais-pack.yaml'), TEST_PACK);
  await writeFile(join(TEST_DIR, 'swap-to-token.ais-flow.yaml'), TEST_WORKFLOW);
});

afterAll(async () => {
  await rm(TEST_DIR, { recursive: true, force: true });
});

describe('Integration: Load Protocols', () => {
  let ctx: ResolverContext;

  beforeAll(async () => {
    const result = await loadDirectoryAsContext(TEST_DIR);
    ctx = result.context;
    expect(result.result.errors).toHaveLength(0);
  });

  it('loads all protocols', () => {
    expect(ctx.protocols.size).toBe(2);
    expect(ctx.protocols.has('uniswap-v3')).toBe(true);
    expect(ctx.protocols.has('erc20')).toBe(true);
  });

  it('resolves protocol metadata', () => {
    const uniswap = ctx.protocols.get('uniswap-v3')!;
    expect(uniswap.meta.name).toBe('Uniswap V3');
    expect(uniswap.meta.version).toBe('1.0.0');
  });

  it('resolves actions', () => {
    const swapAction = resolveAction(ctx, 'uniswap-v3/swap-exact-in');
    expect(swapAction).not.toBeNull();
    expect(swapAction?.action.description).toBe('Swap exact input for maximum output');
  });

  it('resolves queries', () => {
    const query = resolveQuery(ctx, 'uniswap-v3/quote-exact-in');
    expect(query).not.toBeNull();
    expect(query?.query.description).toBe('Get quote for exact input swap');
  });

  it('gets contract addresses for chains', () => {
    const uniswap = ctx.protocols.get('uniswap-v3')!;
    
    const mainnetRouter = getContractAddress(uniswap, 'eip155:1', 'router');
    expect(mainnetRouter).toBe('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');

    const baseRouter = getContractAddress(uniswap, 'eip155:8453', 'router');
    expect(baseRouter).toBe('0x2626664c2603336E57B271c5C0b26F421741e481');

    const unknownChain = getContractAddress(uniswap, 'eip155:999', 'router');
    expect(unknownChain).toBeNull();
  });

  it('gets supported chains', () => {
    const uniswap = ctx.protocols.get('uniswap-v3')!;
    const chains = getSupportedChains(uniswap);
    expect(chains).toContain('eip155:1');
    expect(chains).toContain('eip155:8453');
  });
});

describe('Integration: Pack Operations', () => {
  let ctx: ResolverContext;
  let pack: Pack;

  beforeAll(async () => {
    const result = await loadDirectoryAsContext(TEST_DIR);
    ctx = result.context;
    pack = await loadPack(join(TEST_DIR, 'safe-defi.ais-pack.yaml'));
  });

  it('loads pack metadata', () => {
    expect(pack.name).toBe('safe-defi-pack');
    expect(pack.version).toBe('1.0.0');
  });

  it('expands pack skill references', () => {
    const { protocols, missing } = expandPack(ctx, pack);
    expect(protocols).toHaveLength(2);
    expect(missing).toHaveLength(0);
    expect(protocols.map(p => p.meta.protocol).sort()).toEqual(['erc20', 'uniswap-v3']);
  });

  it('validates constraints - passes valid input', () => {
    const result = validateConstraints(pack.policy, pack.token_policy, {
      token_address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 50,
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(false);
  });

  it('validates constraints - rejects high slippage', () => {
    const result = validateConstraints(pack.policy, pack.token_policy, {
      slippage_bps: 200,
    });
    expect(result.valid).toBe(false);
    expect(result.violations[0].constraint).toContain('max_slippage_bps');
  });

  it('validates constraints - rejects non-allowlisted token', () => {
    const result = validateConstraints(pack.policy, pack.token_policy, {
      token_address: '0x1234567890123456789012345678901234567890',
    });
    expect(result.valid).toBe(false);
  });

  it('validates constraints - requires approval for high risk', () => {
    const result = validateConstraints(pack.policy, pack.token_policy, {
      risk_level: 4,
    });
    expect(result.valid).toBe(true);
    expect(result.requires_approval).toBe(true);
  });
});

describe('Integration: Workflow Operations', () => {
  let ctx: ResolverContext;
  let workflow: Workflow;

  beforeAll(async () => {
    const result = await loadDirectoryAsContext(TEST_DIR);
    ctx = result.context;
    workflow = await loadWorkflow(join(TEST_DIR, 'swap-to-token.ais-flow.yaml'));
  });

  it('loads workflow metadata', () => {
    expect(workflow.meta.name).toBe('swap-to-token');
    expect(workflow.nodes).toHaveLength(4);
  });

  it('extracts workflow protocols', () => {
    const protocols = getWorkflowProtocols(workflow);
    expect(protocols.sort()).toEqual(['erc20', 'uniswap-v3']);
  });

  it('extracts workflow dependencies', () => {
    const deps = getWorkflowDependencies(workflow);
    expect(deps).toContain('erc20@1.0.0/allowance');
    expect(deps).toContain('erc20@1.0.0/approve');
    expect(deps).toContain('uniswap-v3@1.0.0/quote-exact-in');
    expect(deps).toContain('uniswap-v3@1.0.0/swap-exact-in');
  });

  it('validates workflow - all references resolve', () => {
    const result = validateWorkflow(workflow, ctx);
    expect(result.valid).toBe(true);
    expect(result.issues).toHaveLength(0);
  });

  it('resolves expressions', () => {
    setVariable(ctx, 'inputs.target_token', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48');
    setVariable(ctx, 'inputs.amount_in', '1000000000000000000');
    setVariable(ctx, 'ctx.sender', '0xUserAddress');
    setVariable(ctx, 'ctx.router', '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');

    const resolved = resolveExpressionString(
      'Swap ${inputs.amount_in} WETH to ${inputs.target_token} via ${ctx.router}',
      ctx
    );
    expect(resolved).toContain('1000000000000000000');
    expect(resolved).toContain('0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48');
    expect(resolved).toContain('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');
  });
});

describe('Integration: End-to-End Agent Simulation', () => {
  it('simulates complete agent workflow', async () => {
    // 1. Load all available protocols
    const { context: ctx, result } = await loadDirectoryAsContext(TEST_DIR);
    expect(result.errors).toHaveLength(0);
    expect(ctx.protocols.size).toBe(2);

    // 2. Load pack for constraints
    const pack = await loadPack(join(TEST_DIR, 'safe-defi.ais-pack.yaml'));

    // 3. Load workflow to execute
    const workflow = await loadWorkflow(join(TEST_DIR, 'swap-to-token.ais-flow.yaml'));

    // 4. Validate workflow references
    const validation = validateWorkflow(workflow, ctx);
    expect(validation.valid).toBe(true);

    // 5. Check all required protocols are available
    const requiredProtocols = getWorkflowProtocols(workflow);
    const missingProtocols = requiredProtocols.filter(p => !ctx.protocols.has(p));
    expect(missingProtocols).toHaveLength(0);

    // 6. Validate operation against pack constraints
    const constraintCheck = validateConstraints(pack.policy, pack.token_policy, {
      token_address: '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
      slippage_bps: 50,
    });
    expect(constraintCheck.valid).toBe(true);

    // 7. Set up execution context
    setVariable(ctx, 'inputs.target_token', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48');
    setVariable(ctx, 'inputs.amount_in', '1000000000000000000');
    setVariable(ctx, 'ctx.chain', 'eip155:1');
    setVariable(ctx, 'ctx.sender', '0xUserAddress');
    setVariable(ctx, 'ctx.router', '0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');

    // 8. Build execution plan
    const executionPlan = workflow.nodes.map(node => {
      const [protocolName] = node.skill.split('@');
      const protocol = ctx.protocols.get(protocolName)!;

      if (node.type === 'action_ref' && node.action) {
        const action = protocol.actions[node.action];
        const execSpec = action?.execution?.['eip155:*'] || action?.execution?.['*'];
        const contractName = execSpec && 'contract' in execSpec ? execSpec.contract : null;
        const address = contractName ? getContractAddress(protocol, 'eip155:1', contractName) : null;
        return {
          nodeId: node.id,
          type: 'action',
          actionId: node.action,
          contract: address,
        };
      } else if (node.type === 'query_ref' && node.query) {
        return {
          nodeId: node.id,
          type: 'query',
          queryId: node.query,
        };
      }
      return { nodeId: node.id, type: 'unknown' };
    });

    expect(executionPlan).toHaveLength(4);
    expect(executionPlan[0].type).toBe('query');
    expect(executionPlan[1].type).toBe('action');
    expect(executionPlan[2].type).toBe('query');
    expect(executionPlan[3].type).toBe('action');
    expect(executionPlan[3].contract).toBe('0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45');
  });
});
