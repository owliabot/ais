/**
 * Lint command - check AIS files for best practices
 */

import { loadDirectory, parseAIS, type ProtocolSpec, type Pack, type Workflow } from '../../index.js';
import { readFile, stat } from 'node:fs/promises';
import { relative } from 'node:path';
import { formatResults, type CLIOptions, type CLIResult, collectFiles } from '../utils.js';

interface LintRule {
  id: string;
  severity: 'error' | 'warning' | 'info';
  check: (doc: ProtocolSpec | Pack | Workflow, path: string) => LintIssue[];
}

interface LintIssue {
  rule: string;
  severity: 'error' | 'warning' | 'info';
  message: string;
  path?: string;
}

// Lint rules for Protocol Specs
const protocolRules: LintRule[] = [
  {
    id: 'protocol-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais/1.0') return [];
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
      if (doc.schema !== 'ais/1.0') return [];
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
      if (doc.schema !== 'ais/1.0') return [];
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
      if (doc.schema !== 'ais/1.0') return [];
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
      if (doc.schema !== 'ais/1.0') return [];
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

// Lint rules for Packs
const packRules: LintRule[] = [
  {
    id: 'pack-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-pack/1.0') return [];
      if (!doc.description) {
        return [{ rule: 'pack-has-description', severity: 'warning', message: 'Pack should have a description' }];
      }
      return [];
    },
  },
  {
    id: 'pack-has-includes',
    severity: 'error',
    check: (doc) => {
      if (doc.schema !== 'ais-pack/1.0') return [];
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
      if (doc.schema !== 'ais-pack/1.0') return [];
      const issues: LintIssue[] = [];
      for (let i = 0; i < doc.includes.length; i++) {
        const ref = doc.includes[i];
        if (!ref.includes('@')) {
          issues.push({
            rule: 'pack-skill-ref-format',
            severity: 'warning',
            message: `Skill reference '${ref}' should include version (e.g., '${ref}@1.0.0')`,
            path: `includes[${i}]`,
          });
        }
      }
      return issues;
    },
  },
];

// Lint rules for Workflows
const workflowRules: LintRule[] = [
  {
    id: 'workflow-has-description',
    severity: 'warning',
    check: (doc) => {
      if (doc.schema !== 'ais-flow/1.0') return [];
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
      if (doc.schema !== 'ais-flow/1.0') return [];
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
      if (doc.schema !== 'ais-flow/1.0') return [];
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
      if (doc.schema !== 'ais-flow/1.0') return [];
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

function lintDocument(doc: ProtocolSpec | Pack | Workflow): LintIssue[] {
  const issues: LintIssue[] = [];
  const allRules = [...protocolRules, ...packRules, ...workflowRules];

  for (const rule of allRules) {
    issues.push(...rule.check(doc, ''));
  }

  return issues;
}

export async function lintCommand(options: CLIOptions, useColor: boolean): Promise<void> {
  const results: CLIResult[] = [];
  let hasErrors = false;
  let hasWarnings = false;

  for (const inputPath of options.paths) {
    const stats = await stat(inputPath).catch(() => null);

    if (!stats) {
      results.push({
        path: inputPath,
        type: 'error',
        message: 'Path does not exist',
      });
      hasErrors = true;
      continue;
    }

    if (stats.isDirectory()) {
      const dirResult = await loadDirectory(inputPath, { recursive: options.recursive });

      // Lint protocols
      for (const { path, document } of dirResult.protocols) {
        const issues = lintDocument(document);
        if (issues.length === 0) {
          results.push({
            path: relative(process.cwd(), path),
            type: 'success',
            message: 'No lint issues',
          });
        } else {
          for (const issue of issues) {
            results.push({
              path: relative(process.cwd(), path),
              type: issue.severity,
              message: `[${issue.rule}] ${issue.message}`,
            });
            if (issue.severity === 'error') hasErrors = true;
            if (issue.severity === 'warning') hasWarnings = true;
          }
        }
      }

      // Lint packs
      for (const { path, document } of dirResult.packs) {
        const issues = lintDocument(document);
        if (issues.length === 0) {
          results.push({
            path: relative(process.cwd(), path),
            type: 'success',
            message: 'No lint issues',
          });
        } else {
          for (const issue of issues) {
            results.push({
              path: relative(process.cwd(), path),
              type: issue.severity,
              message: `[${issue.rule}] ${issue.message}`,
            });
            if (issue.severity === 'error') hasErrors = true;
            if (issue.severity === 'warning') hasWarnings = true;
          }
        }
      }

      // Lint workflows
      for (const { path, document } of dirResult.workflows) {
        const issues = lintDocument(document);
        if (issues.length === 0) {
          results.push({
            path: relative(process.cwd(), path),
            type: 'success',
            message: 'No lint issues',
          });
        } else {
          for (const issue of issues) {
            results.push({
              path: relative(process.cwd(), path),
              type: issue.severity,
              message: `[${issue.rule}] ${issue.message}`,
            });
            if (issue.severity === 'error') hasErrors = true;
            if (issue.severity === 'warning') hasWarnings = true;
          }
        }
      }

      // Report parse errors
      for (const { path, error } of dirResult.errors) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'error',
          message: `Parse error: ${error}`,
        });
        hasErrors = true;
      }
    } else {
      // Single file
      const files = await collectFiles([inputPath]);
      
      for (const file of files) {
        try {
          const content = await readFile(file, 'utf-8');
          const doc = parseAIS(content, { source: file });
          const issues = lintDocument(doc);

          if (issues.length === 0) {
            results.push({
              path: relative(process.cwd(), file),
              type: 'success',
              message: 'No lint issues',
            });
          } else {
            for (const issue of issues) {
              results.push({
                path: relative(process.cwd(), file),
                type: issue.severity,
                message: `[${issue.rule}] ${issue.message}`,
              });
              if (issue.severity === 'error') hasErrors = true;
              if (issue.severity === 'warning') hasWarnings = true;
            }
          }
        } catch (err) {
          results.push({
            path: relative(process.cwd(), file),
            type: 'error',
            message: err instanceof Error ? err.message : String(err),
          });
          hasErrors = true;
        }
      }
    }
  }

  // Output results
  formatResults(results, options, useColor, 'Lint');

  // Exit with appropriate code
  process.exit(hasErrors ? 1 : hasWarnings ? 0 : 0);
}
