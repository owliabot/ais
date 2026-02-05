/**
 * Constraint validation - validate values against Pack policy constraints
 */
import type { Policy, HardConstraintsDefaults, TokenPolicy } from '../schema/index.js';

export interface ConstraintInput {
  /** Token address being used */
  token_address?: string;
  /** Token symbol */
  token_symbol?: string;
  /** Chain ID (CAIP-2) */
  chain?: string;
  /** Amount being spent (as string, e.g., "100 USDC") */
  spend_amount?: string;
  /** Approval amount (as string) */
  approval_amount?: string;
  /** Slippage in basis points */
  slippage_bps?: number;
  /** Whether this is an unlimited approval */
  unlimited_approval?: boolean;
  /** Risk level of the action (1-5) */
  risk_level?: number;
  /** Risk tags of the action */
  risk_tags?: string[];
}

export interface ConstraintViolation {
  field: string;
  message: string;
  constraint: string;
  value: unknown;
  limit: unknown;
}

export interface ConstraintResult {
  valid: boolean;
  violations: ConstraintViolation[];
  requires_approval: boolean;
  approval_reasons: string[];
}

/**
 * Validate input values against Pack policy
 */
export function validateConstraints(
  policy: Policy | undefined,
  tokenPolicy: TokenPolicy | undefined,
  input: ConstraintInput
): ConstraintResult {
  const violations: ConstraintViolation[] = [];
  const approvalReasons: string[] = [];

  // Check token policy
  if (tokenPolicy?.allowlist && tokenPolicy.allowlist.length > 0) {
    const tokenToCheck = input.token_address?.toLowerCase() ?? input.token_symbol?.toLowerCase();
    
    if (tokenToCheck) {
      const isAllowed = tokenPolicy.allowlist.some((entry) => {
        // Match by address or symbol
        const matchesAddress = entry.address.toLowerCase() === tokenToCheck;
        const matchesSymbol = entry.symbol.toLowerCase() === tokenToCheck;
        // Also check chain if provided
        const matchesChain = !input.chain || entry.chain === input.chain;
        return (matchesAddress || matchesSymbol) && matchesChain;
      });

      if (!isAllowed) {
        const resolution = tokenPolicy.resolution;
        const strictMode =
          resolution?.require_allowlist_for_symbol_resolution ||
          (resolution && 'strict' in resolution);

        if (strictMode) {
          violations.push({
            field: 'token',
            message: `Token ${tokenToCheck} not in allowlist`,
            constraint: 'token_policy.allowlist',
            value: tokenToCheck,
            limit: tokenPolicy.allowlist.map((e) => `${e.symbol} (${e.chain})`),
          });
        } else {
          approvalReasons.push(`Token ${tokenToCheck} not in allowlist`);
        }
      }
    }
  }

  // Check hard constraints (from hard_constraints_defaults or hard_constraints)
  const hc = policy?.hard_constraints_defaults ?? policy?.hard_constraints;
  if (hc) {
    // Slippage check
    if (input.slippage_bps !== undefined && hc.max_slippage_bps !== undefined) {
      if (input.slippage_bps > hc.max_slippage_bps) {
        violations.push({
          field: 'slippage_bps',
          message: `Slippage ${input.slippage_bps} bps exceeds max ${hc.max_slippage_bps} bps`,
          constraint: 'hard_constraints.max_slippage_bps',
          value: input.slippage_bps,
          limit: hc.max_slippage_bps,
        });
      }
    }

    // Unlimited approval check
    if (input.unlimited_approval && hc.allow_unlimited_approval === false) {
      violations.push({
        field: 'unlimited_approval',
        message: 'Unlimited approvals are not allowed',
        constraint: 'hard_constraints.allow_unlimited_approval',
        value: true,
        limit: false,
      });
    }
  }

  // Check approval policy
  const approvals = policy?.approvals;
  if (approvals && input.risk_level !== undefined) {
    if (
      approvals.auto_execute_max_risk_level !== undefined &&
      input.risk_level > approvals.auto_execute_max_risk_level
    ) {
      approvalReasons.push(
        `Risk level ${input.risk_level} exceeds auto-execute threshold ${approvals.auto_execute_max_risk_level}`
      );
    }
    if (
      approvals.require_approval_min_risk_level !== undefined &&
      input.risk_level >= approvals.require_approval_min_risk_level
    ) {
      approvalReasons.push(
        `Risk level ${input.risk_level} requires approval (min: ${approvals.require_approval_min_risk_level})`
      );
    }
  }

  // Legacy: Check risk threshold
  if (policy?.risk_threshold !== undefined && input.risk_level !== undefined) {
    if (input.risk_level > policy.risk_threshold) {
      approvalReasons.push(
        `Risk level ${input.risk_level} exceeds auto-approve threshold ${policy.risk_threshold}`
      );
    }
  }

  // Legacy: Check risk tags requiring approval
  if (policy?.approval_required && input.risk_tags) {
    const requiringApproval = input.risk_tags.filter((tag) =>
      policy.approval_required!.includes(tag)
    );
    if (requiringApproval.length > 0) {
      approvalReasons.push(
        `Risk tags require approval: ${requiringApproval.join(', ')}`
      );
    }
  }

  return {
    valid: violations.length === 0,
    violations,
    requires_approval: approvalReasons.length > 0,
    approval_reasons: approvalReasons,
  };
}

/**
 * Extract hard constraints from policy (for display/UI)
 */
export function getHardConstraints(
  policy: Policy | undefined
): HardConstraintsDefaults {
  return policy?.hard_constraints_defaults ?? policy?.hard_constraints ?? {};
}
