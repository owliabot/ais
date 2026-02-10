/**
 * Lint command - check AIS files for best practices
 */

import { loadDirectory, parseAIS, type ProtocolSpec, type Pack, type Workflow } from '../../index.js';
import { lintDocument } from '../../validator/index.js';
import { readFile, stat } from 'node:fs/promises';
import { relative } from 'node:path';
import { formatResults, type CLIOptions, type CLIResult, collectFiles } from '../utils.js';

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
        const issues = lintDocument(document, { file_path: path });
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
        const issues = lintDocument(document, { file_path: path });
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
        const issues = lintDocument(document, { file_path: path });
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
      for (const e of dirResult.errors) {
        const base = e.kind ? `[${e.kind}] ${e.error}` : e.error;
        if (e.issues && e.issues.length > 0) {
          for (const issue of e.issues) {
            results.push({
              path: relative(process.cwd(), e.path),
              type: 'error',
              message: `Parse error: ${base} (${issue.path}: ${issue.message})`,
            });
          }
        } else if (e.field_path) {
          results.push({
            path: relative(process.cwd(), e.path),
            type: 'error',
            message: `Parse error: ${base} (${e.field_path})`,
          });
        } else {
          results.push({
            path: relative(process.cwd(), e.path),
            type: 'error',
            message: `Parse error: ${base}`,
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
          const issues = lintDocument(doc, { file_path: file });

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
