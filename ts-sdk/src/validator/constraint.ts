/**
 * Constraint validation - validate values against Pack constraints
 */
import type { PackConstraints } from '../schema/index.js';

export interface ConstraintInput {
  /** Token address being used */
  token?: string;
  /** Amount in USD */
  amount_usd?: number;
  /** Percentage of balance being used */
  percentage_of_balance?: number;
  /** Slippage in basis points */
  slippage_bps?: number;
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
}

/**
 * Validate input values against Pack constraints
 */
export function validateConstraints(
  constraints: PackConstraints | undefined,
  input: ConstraintInput
): ConstraintResult {
  const violations: ConstraintViolation[] = [];

  if (!constraints) {
    return { valid: true, violations: [] };
  }

  // Token allowlist/blocklist
  if (input.token && constraints.tokens) {
    const token = input.token.toLowerCase();

    if (constraints.tokens.allowlist) {
      const allowed = constraints.tokens.allowlist.map((t) => t.toLowerCase());
      if (!allowed.includes(token)) {
        violations.push({
          field: 'token',
          message: `Token ${input.token} not in allowlist`,
          constraint: 'tokens.allowlist',
          value: input.token,
          limit: constraints.tokens.allowlist,
        });
      }
    }

    if (constraints.tokens.blocklist) {
      const blocked = constraints.tokens.blocklist.map((t) => t.toLowerCase());
      if (blocked.includes(token)) {
        violations.push({
          field: 'token',
          message: `Token ${input.token} is blocklisted`,
          constraint: 'tokens.blocklist',
          value: input.token,
          limit: constraints.tokens.blocklist,
        });
      }
    }
  }

  // Amount constraints
  if (constraints.amounts) {
    if (
      input.amount_usd !== undefined &&
      constraints.amounts.max_usd !== undefined
    ) {
      if (input.amount_usd > constraints.amounts.max_usd) {
        violations.push({
          field: 'amount_usd',
          message: `Amount $${input.amount_usd} exceeds max $${constraints.amounts.max_usd}`,
          constraint: 'amounts.max_usd',
          value: input.amount_usd,
          limit: constraints.amounts.max_usd,
        });
      }
    }

    if (
      input.percentage_of_balance !== undefined &&
      constraints.amounts.max_percentage_of_balance !== undefined
    ) {
      if (
        input.percentage_of_balance >
        constraints.amounts.max_percentage_of_balance
      ) {
        violations.push({
          field: 'percentage_of_balance',
          message: `${input.percentage_of_balance}% exceeds max ${constraints.amounts.max_percentage_of_balance}%`,
          constraint: 'amounts.max_percentage_of_balance',
          value: input.percentage_of_balance,
          limit: constraints.amounts.max_percentage_of_balance,
        });
      }
    }
  }

  // Slippage constraint
  if (
    input.slippage_bps !== undefined &&
    constraints.slippage?.max_bps !== undefined
  ) {
    if (input.slippage_bps > constraints.slippage.max_bps) {
      violations.push({
        field: 'slippage_bps',
        message: `Slippage ${input.slippage_bps} bps exceeds max ${constraints.slippage.max_bps} bps`,
        constraint: 'slippage.max_bps',
        value: input.slippage_bps,
        limit: constraints.slippage.max_bps,
      });
    }
  }

  return {
    valid: violations.length === 0,
    violations,
  };
}

/**
 * Check if simulation is required by constraints
 */
export function requiresSimulation(
  constraints: PackConstraints | undefined
): boolean {
  return constraints?.require_simulation === true;
}
