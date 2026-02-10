import type { ProtocolSpec, Pack, Workflow } from '../schema/index.js';
import type { ValidatorRegistry, LintIssue, LintRule } from './plugins.js';
import { defaultValidatorRegistry } from './plugins.js';

// Built-in lint rules (best-practice guidance; not normative validation)
const builtinProtocolRules: LintRule[] = [
  {
    id: 'protocol-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais/0.0.2') return [];
      if (!doc.meta.description) {
        return [{ rule: 'protocol-has-description', severity: 'warning', message: 'Protocol should have a description' }];
      }
      return [];
    },
  },
  {
    id: 'protocol-has-deployments',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais/0.0.2') return [];
      if (doc.deployments.length === 0) {
        return [{ rule: 'protocol-has-deployments', severity: 'error', message: 'Protocol must have at least one deployment' }];
      }
      return [];
    },
  },
  {
    id: 'action-has-description',
    severity: 'info',
    check: (doc) => {
      if (doc.schema !== 'ais/0.0.2') return [];
      const issues: LintIssue[] = [];
      for (const [name, action] of Object.entries(doc.actions)) {
        if (!action.description) {
          issues.push({
            rule: 'action-has-description',
            severity: 'info',
            message: `Action '${name}' should have a description`,
            path: `actions.${name}`,
          });
        }
      }
      return issues;
    },
  },
  {
    id: 'action-has-params',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais/0.0.2') return [];
      const issues: LintIssue[] = [];
      for (const [name, action] of Object.entries(doc.actions)) {
        if (!action.params || action.params.length === 0) {
          issues.push({
            rule: 'action-has-params',
            severity: 'warning',
            message: `Action '${name}' has no parameters defined`,
            path: `actions.${name}`,
          });
        }
      }
      return issues;
    },
  },
  {
    id: 'contract-address-format',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais/0.0.2') return [];
      const issues: LintIssue[] = [];
      for (let i = 0; i < doc.deployments.length; i++) {
        const deployment = doc.deployments[i];
        for (const [name, address] of Object.entries(deployment.contracts)) {
          if (!/^0x[a-fA-F0-9]{40}$/.test(address)) {
            issues.push({
              rule: 'contract-address-format',
              severity: 'error',
              message: `Invalid contract address for '${name}': ${address}`,
              path: `deployments[${i}].contracts.${name}`,
            });
          }
        }
      }
      return issues;
    },
  },
];

const builtinPackRules: LintRule[] = [
  {
    id: 'pack-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-pack/0.0.2') return [];
      const desc = doc.meta?.description ?? (doc as any).description;
      if (!desc) {
        return [{ rule: 'pack-has-description', severity: 'warning', message: 'Pack should have a description' }];
      }
      return [];
    },
  },
  {
    id: 'pack-has-includes',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais-pack/0.0.2') return [];
      if (doc.includes.length === 0) {
        return [{ rule: 'pack-has-includes', severity: 'error', message: 'Pack must include at least one protocol' }];
      }
      return [];
    },
  },
  {
    id: 'pack-skill-ref-format',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-pack/0.0.2') return [];
      const issues: LintIssue[] = [];
      for (let i = 0; i < doc.includes.length; i++) {
        const ref = doc.includes[i];
        if (!ref.version) {
          issues.push({
            rule: 'pack-skill-ref-format',
            severity: 'warning',
            message: `Skill reference '${ref.protocol}' should include version`,
            path: `includes[${i}]`,
          });
        }
      }
      return issues;
    },
  },
];

const builtinWorkflowRules: LintRule[] = [
  {
    id: 'workflow-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-flow/0.0.2') return [];
      if (!doc.meta.description) {
        return [{ rule: 'workflow-has-description', severity: 'warning', message: 'Workflow should have a description' }];
      }
      return [];
    },
  },
  {
    id: 'workflow-has-nodes',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais-flow/0.0.2') return [];
      if (doc.nodes.length === 0) {
        return [{ rule: 'workflow-has-nodes', severity: 'error', message: 'Workflow must have at least one node' }];
      }
      return [];
    },
  },
  {
    id: 'workflow-node-has-id',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais-flow/0.0.2') return [];
      const issues: LintIssue[] = [];
      const ids = new Set<string>();
      for (let i = 0; i < doc.nodes.length; i++) {
        const node = doc.nodes[i];
        if (!node.id) {
          issues.push({
            rule: 'workflow-node-has-id',
            severity: 'error',
            message: `Node at index ${i} must have an id`,
            path: `nodes[${i}]`,
          });
        } else if (ids.has(node.id)) {
          issues.push({
            rule: 'workflow-node-has-id',
            severity: 'error',
            message: `Duplicate node id: '${node.id}'`,
            path: `nodes[${i}].id`,
          });
        }
        ids.add(node.id);
      }
      return issues;
    },
  },
  {
    id: 'workflow-node-skill-ref-format',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-flow/0.0.2') return [];
      const issues: LintIssue[] = [];
      for (let i = 0; i < doc.nodes.length; i++) {
        const node = doc.nodes[i];
        if (!node.skill.includes('@')) {
          issues.push({
            rule: 'workflow-node-skill-ref-format',
            severity: 'warning',
            message: `Node '${node.id}' skill reference should include version`,
            path: `nodes[${i}].skill`,
          });
        }
      }
      return issues;
    },
  },
];

export function lintDocument(
  doc: ProtocolSpec | Pack | Workflow,
  options: { file_path?: string; registry?: ValidatorRegistry } = {}
): LintIssue[] {
  const registry = options.registry ?? defaultValidatorRegistry;
  const filePath = options.file_path;

  const builtinRules = [...builtinProtocolRules, ...builtinPackRules, ...builtinWorkflowRules];
  const pluginRules = registry.listLintRules();
  const allRules = [...builtinRules, ...pluginRules];

  const issues: LintIssue[] = [];
  for (const rule of allRules) {
    issues.push(...rule.check(doc, filePath));
  }
  return issues;
}
