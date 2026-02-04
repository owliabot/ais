/**
 * Validate command - validate AIS files against schema
 */

import { loadDirectory, validate } from '../../index.js';
import { readFile, stat } from 'node:fs/promises';
import { relative } from 'node:path';
import { formatResults, type CLIOptions, type CLIResult, collectFiles } from '../utils.js';

export async function validateCommand(options: CLIOptions, useColor: boolean): Promise<void> {
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
      // Load all files from directory
      const dirResult = await loadDirectory(inputPath, { recursive: options.recursive });

      // Report successfully loaded files
      for (const { path, document } of dirResult.protocols) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `Valid protocol: ${document.meta.protocol}@${document.meta.version}`,
          document,
        });
      }

      for (const { path, document } of dirResult.packs) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `Valid pack: ${document.name}@${document.version}`,
          document,
        });
      }

      for (const { path, document } of dirResult.workflows) {
        results.push({
          path: relative(process.cwd(), path),
          type: 'success',
          message: `Valid workflow: ${document.meta.name}@${document.meta.version}`,
          document,
        });
      }

      // Report errors
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
          const validation = validate(content);

          if (validation.valid) {
            results.push({
              path: relative(process.cwd(), file),
              type: 'success',
              message: 'Valid AIS document',
            });
          } else {
            for (const issue of validation.issues) {
              results.push({
                path: relative(process.cwd(), file),
                type: 'error',
                message: issue,
              });
              hasErrors = true;
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
  formatResults(results, options, useColor, 'Validation');

  // Exit with appropriate code
  process.exit(hasErrors ? 1 : 0);
}
