/**
 * CLI utilities
 */

import { stat, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { AnyAISDocument } from '../index.js';

export interface CLIOptions {
  recursive: boolean;
  quiet: boolean;
  verbose: boolean;
  json: boolean;
  noColor: boolean;
  paths: string[];
}

export interface CLIResult {
  path: string;
  type: 'success' | 'error' | 'warning' | 'info';
  message: string;
  document?: AnyAISDocument;
}

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  gray: '\x1b[90m',
  bold: '\x1b[1m',
};

function colorize(text: string, color: keyof typeof colors, useColor: boolean): string {
  if (!useColor) return text;
  return `${colors[color]}${text}${colors.reset}`;
}

export function formatResults(
  results: CLIResult[],
  options: CLIOptions,
  useColor: boolean,
  title: string
): void {
  if (options.json) {
    // JSON output
    const output = {
      title,
      summary: {
        total: results.length,
        success: results.filter((r) => r.type === 'success').length,
        errors: results.filter((r) => r.type === 'error').length,
        warnings: results.filter((r) => r.type === 'warning').length,
        info: results.filter((r) => r.type === 'info').length,
      },
      results: results.map((r) => ({
        path: r.path,
        type: r.type,
        message: r.message,
      })),
    };
    console.log(JSON.stringify(output, null, 2));
    return;
  }

  // Text output
  const errorCount = results.filter((r) => r.type === 'error').length;
  const warningCount = results.filter((r) => r.type === 'warning').length;
  const successCount = results.filter((r) => r.type === 'success').length;

  // Group results by path
  const byPath = new Map<string, CLIResult[]>();
  for (const result of results) {
    const existing = byPath.get(result.path) ?? [];
    existing.push(result);
    byPath.set(result.path, existing);
  }

  // Print header
  if (!options.quiet) {
    console.log();
    console.log(colorize(`${title} Results`, 'bold', useColor));
    console.log(colorize('─'.repeat(50), 'gray', useColor));
  }

  // Print results
  for (const [path, pathResults] of byPath) {
    const hasError = pathResults.some((r) => r.type === 'error');
    const hasWarning = pathResults.some((r) => r.type === 'warning');

    // In quiet mode, only show files with errors
    if (options.quiet && !hasError) continue;

    // Print path header
    const pathColor = hasError ? 'red' : hasWarning ? 'yellow' : 'green';
    console.log();
    console.log(colorize(path, pathColor, useColor));

    // Print issues for this path
    for (const result of pathResults) {
      // In quiet mode, only show errors
      if (options.quiet && result.type !== 'error') continue;

      // Skip success messages unless verbose
      if (!options.verbose && result.type === 'success') continue;

      const icon = getIcon(result.type, useColor);
      const msgColor = getTypeColor(result.type);
      console.log(`  ${icon} ${colorize(result.message, msgColor, useColor)}`);
    }
  }

  // Print summary
  if (!options.quiet) {
    console.log();
    console.log(colorize('─'.repeat(50), 'gray', useColor));

    const parts: string[] = [];
    if (successCount > 0) {
      parts.push(colorize(`${successCount} passed`, 'green', useColor));
    }
    if (warningCount > 0) {
      parts.push(colorize(`${warningCount} warnings`, 'yellow', useColor));
    }
    if (errorCount > 0) {
      parts.push(colorize(`${errorCount} errors`, 'red', useColor));
    }

    console.log(`Summary: ${parts.join(', ')}`);
    console.log();
  }
}

function getIcon(type: CLIResult['type'], useColor: boolean): string {
  switch (type) {
    case 'success':
      return colorize('✓', 'green', useColor);
    case 'error':
      return colorize('✗', 'red', useColor);
    case 'warning':
      return colorize('⚠', 'yellow', useColor);
    case 'info':
      return colorize('ℹ', 'blue', useColor);
  }
}

function getTypeColor(type: CLIResult['type']): keyof typeof colors {
  switch (type) {
    case 'success':
      return 'green';
    case 'error':
      return 'red';
    case 'warning':
      return 'yellow';
    case 'info':
      return 'blue';
  }
}

/**
 * Collect all AIS files from paths (handles globs and directories)
 */
export async function collectFiles(paths: string[]): Promise<string[]> {
  const files: string[] = [];

  for (const path of paths) {
    const stats = await stat(path).catch(() => null);
    if (!stats) continue;

    if (stats.isFile() && isAISFile(path)) {
      files.push(path);
    } else if (stats.isDirectory()) {
      const entries = await readdir(path);
      for (const entry of entries) {
        const fullPath = join(path, entry);
        const entryStats = await stat(fullPath).catch(() => null);
        if (entryStats?.isFile() && isAISFile(fullPath)) {
          files.push(fullPath);
        }
      }
    }
  }

  return files;
}

function isAISFile(filename: string): boolean {
  return (
    filename.endsWith('.ais.yaml') ||
    filename.endsWith('.ais.yml') ||
    filename.endsWith('.ais-pack.yaml') ||
    filename.endsWith('.ais-pack.yml') ||
    filename.endsWith('.ais-flow.yaml') ||
    filename.endsWith('.ais-flow.yml')
  );
}
