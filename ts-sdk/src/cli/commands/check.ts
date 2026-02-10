/**
 * Check command - run all checks (validate + lint + workflow validation)
 */

import {
  loadDirectoryAsContext,
  parseAIS,
  validateWorkflow,
  validateWorkspaceReferences,
} from '../../index.js';
import { readFile, stat } from 'node:fs/promises';
import { relative } from 'node:path';
import { formatResults, type CLIOptions, type CLIResult, collectFiles } from '../utils.js';

export async function checkCommand(options: CLIOptions, useColor: boolean): Promise<void> {
  const results: CLIResult[] = [];
  let hasErrors = false;

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
      // Load all files and create context for workflow validation
      const { context, result: dirResult } = await loadDirectoryAsContext(inputPath, {
        recursive: options.recursive,
      });

      // Cross-file checks (workflow → pack → protocol)
      const wsIssues = validateWorkspaceReferences({
        protocols: dirResult.protocols,
        packs: dirResult.packs,
        workflows: dirResult.workflows,
      });
      for (const issue of wsIssues) {
        const msgParts: string[] = [];
        if (issue.field_path) msgParts.push(`${issue.field_path}:`);
        msgParts.push(issue.message);
        if (issue.related_path) msgParts.push(`(related: ${relative(process.cwd(), issue.related_path)})`);
        results.push({
          path: relative(process.cwd(), issue.path),
          type: issue.severity === 'warning' ? 'warning' : issue.severity === 'info' ? 'info' : 'error',
          message: msgParts.join(' '),
        });
        if (issue.severity === 'error') hasErrors = true;
      }

      // Report schema validation results
      for (const { path, document } of dirResult.protocols) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `✓ Schema valid: ${document.meta.protocol}@${document.meta.version}`,
          document,
        });

        // Additional checks for protocols
        if (document.deployments.length === 0) {
          results.push({
            path: relative(process.cwd(), path),
            type: 'warning',
            message: 'No deployments defined',
          });
        }

        // Check contract addresses
        for (const deployment of document.deployments) {
          for (const [name, address] of Object.entries(deployment.contracts)) {
            if (!/^0x[a-fA-F0-9]{40}$/.test(address)) {
              results.push({
                path: relative(process.cwd(), path),
                type: 'error',
                message: `Invalid address for ${name}: ${address}`,
              });
              hasErrors = true;
            }
          }
        }
      }

      for (const { path, document } of dirResult.packs) {
        const name = document.meta?.name ?? document.name ?? '(unknown-pack)';
        const version = document.meta?.version ?? document.version ?? '(unknown-version)';
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `✓ Schema valid: ${name}@${version}`,
          document,
        });
      }

      for (const { path, document } of dirResult.workflows) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `✓ Schema valid: ${document.meta.name}@${document.meta.version}`,
          document,
        });

        // Validate workflow references
        const validation = validateWorkflow(document, context);
        if (!validation.valid) {
          for (const issue of validation.issues) {
            results.push({
              path: relative(process.cwd(), path),
              type: 'error',
              message: `Node '${issue.nodeId}' (${issue.field}): ${issue.message}${issue.reference ? ` [ref=${issue.reference}]` : ''}`,
            });
            hasErrors = true;
          }
        } else {
          results.push({
            path: relative(process.cwd(), path),
            type: 'success',
            message: '✓ Workflow references valid',
          });
        }
      }

      // Report parse errors
      for (const e of dirResult.errors) {
        const base = e.kind ? `[${e.kind}] ${e.error}` : e.error;
        if (e.issues && e.issues.length > 0) {
          for (const issue of e.issues) {
            results.push({
              path: relative(process.cwd(), e.path),
              type: 'error',
              message: `${base} (${issue.path}: ${issue.message})`,
            });
          }
        } else if (e.field_path) {
          results.push({
            path: relative(process.cwd(), e.path),
            type: 'error',
            message: `${base} (${e.field_path})`,
          });
        } else {
          results.push({
            path: relative(process.cwd(), e.path),
            type: 'error',
            message: base,
          });
        }
        hasErrors = true;
      }
    } else {
      // Single file
      const files = await collectFiles([inputPath]);

      for (const file of files) {
        try {
          const content = await readFile(file, 'utf-8');
          const doc = parseAIS(content, { source: file });

          results.push({
            path: relative(process.cwd(), file),
            type: 'success',
            message: `✓ Schema valid`,
          });

          // Type-specific checks
          if (doc.schema === 'ais/0.0.2') {
            for (const deployment of doc.deployments) {
              for (const [name, address] of Object.entries(deployment.contracts)) {
                if (!/^0x[a-fA-F0-9]{40}$/.test(address)) {
                  results.push({
                    path: relative(process.cwd(), file),
                    type: 'error',
                    message: `Invalid address for ${name}: ${address}`,
                  });
                  hasErrors = true;
                }
              }
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
  formatResults(results, options, useColor, 'Check');

  // Exit with appropriate code
  process.exit(hasErrors ? 1 : 0);
}
