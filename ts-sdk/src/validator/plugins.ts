import type { ProtocolSpec, Pack, Workflow } from '../schema/index.js';
import type { ResolverContext } from '../resolver/index.js';
import type { WorkflowIssue } from './workflow.js';

export type LintSeverity = 'error' | 'warning' | 'info';

export interface LintIssue {
  rule: string;
  severity: LintSeverity;
  message: string;
  path?: string;
}

export interface LintRule {
  id: string;
  severity: LintSeverity;
  check: (doc: ProtocolSpec | Pack | Workflow, filePath?: string) => LintIssue[];
}

export interface ValidatorPlugin {
  id: string;
  lint_rules?: LintRule[];
  validate_workflow?: (workflow: Workflow, ctx: ResolverContext) => WorkflowIssue[];
}

export class ValidatorRegistry {
  private readonly lintRules: LintRule[] = [];
  private readonly workflowValidators: Array<(workflow: Workflow, ctx: ResolverContext) => WorkflowIssue[]> = [];

  register(plugin: ValidatorPlugin): void {
    if (!plugin.id) throw new Error('ValidatorPlugin.id is required');
    for (const r of plugin.lint_rules ?? []) this.lintRules.push(r);
    if (plugin.validate_workflow) this.workflowValidators.push(plugin.validate_workflow);
  }

  listLintRules(): LintRule[] {
    return this.lintRules.slice();
  }

  listWorkflowValidators(): Array<(workflow: Workflow, ctx: ResolverContext) => WorkflowIssue[]> {
    return this.workflowValidators.slice();
  }
}

export const defaultValidatorRegistry = new ValidatorRegistry();

export function createValidatorRegistry(): ValidatorRegistry {
  return new ValidatorRegistry();
}

export function registerValidatorPlugin(plugin: ValidatorPlugin): void {
  defaultValidatorRegistry.register(plugin);
}

