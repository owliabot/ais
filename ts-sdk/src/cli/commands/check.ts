/**
 * Check command - run all checks (validate + lint + workflow validation)
 */

import {
  loadDirectoryAsContext,
  parseAIS,
  validateWorkflow,
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
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `✓ Schema valid: ${document.name}@${document.version}`,
          document,
        });

        // Check skill references
        for (const skillInclude of document.includes) {
          if (!context.protocols.has(skillInclude.protocol)) {
            results.push({
              path: relative(process.cwd(), path),
              type: 'warning',
              message: `Referenced protocol not found locally: ${skillInclude.protocol}@${skillInclude.version}`,
            });
          }
        }
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
              message: `Node '${issue.nodeId}': ${issue.message}`,
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
      for (const { path, error } of dirResult.errors) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'error',
          message: error,
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

          results.push({
            path: relative(process.cwd(), file),
            type: 'success',
            message: `✓ Schema valid`,
          });

          // Type-specific checks
          if (doc.schema === 'ais/1.0') {
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
