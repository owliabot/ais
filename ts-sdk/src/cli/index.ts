#!/usr/bin/env node
/**
 * AIS CLI - Command line tool for validating and linting AIS files
 */

import { resolve } from 'node:path';
import { validateCommand } from './commands/validate.js';
import { lintCommand } from './commands/lint.js';
import { checkCommand } from './commands/check.js';
import { catalogCommand } from './commands/catalog.js';

const VERSION = '0.1.0';

const HELP = `
AIS CLI - Agent Interaction Specification Tools

Usage: ais <command> [options] [path...]

Commands:
  validate <path...>   Validate AIS files against schema
  lint <path...>       Lint AIS files for best practices
  check <path...>      Run all checks (validate + lint)
  catalog <dir>        Export workspace catalog cards as JSON
  help                 Show this help message
  version              Show version

Options:
  -r, --recursive      Process directories recursively (default: true)
  -q, --quiet          Only show errors
  -v, --verbose        Show detailed output
  --json               Output results as JSON
  --out <path>         Write catalog JSON to a file (catalog only)
  --pretty             Pretty-print JSON output (catalog only)
  --no-color           Disable colored output

Examples:
  ais validate ./protocols/
  ais lint ./specs/*.ais.yaml
  ais check . --recursive
  ais validate protocol.ais.yaml --json

Documentation: https://docs.openclaw.ai/ais
`;

interface CLIOptions {
  recursive: boolean;
  quiet: boolean;
  verbose: boolean;
  json: boolean;
  noColor: boolean;
  outPath?: string;
  pretty?: boolean;
  paths: string[];
}

function parseArgs(args: string[]): { command: string; options: CLIOptions } {
  const options: CLIOptions = {
    recursive: true,
    quiet: false,
    verbose: false,
    json: false,
    noColor: false,
    paths: [],
  };

  let command = 'help';
  let i = 0;

  // First non-flag argument is the command
  if (args.length > 0 && !args[0].startsWith('-')) {
    command = args[0];
    i = 1;
  }

  // Parse remaining arguments
  while (i < args.length) {
    const arg = args[i];

    switch (arg) {
      case '-r':
      case '--recursive':
        options.recursive = true;
        break;
      case '--no-recursive':
        options.recursive = false;
        break;
      case '-q':
      case '--quiet':
        options.quiet = true;
        break;
      case '-v':
      case '--verbose':
        options.verbose = true;
        break;
      case '--json':
        options.json = true;
        break;
      case '--no-color':
        options.noColor = true;
        break;
      case '--out':
        i++;
        options.outPath = args[i];
        break;
      case '--pretty':
        options.pretty = true;
        break;
      default:
        if (arg.startsWith('-')) {
          console.error(`Unknown option: ${arg}`);
          process.exit(1);
        }
        options.paths.push(resolve(arg));
    }
    i++;
  }

  // Default to current directory if no paths specified
  if (options.paths.length === 0) {
    options.paths.push(resolve('.'));
  }

  return { command, options };
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const { command, options } = parseArgs(args);

  // Disable colors if requested or not a TTY
  const useColor = !options.noColor && process.stdout.isTTY;

  try {
    switch (command) {
      case 'validate':
        await validateCommand(options, useColor);
        break;

      case 'lint':
        await lintCommand(options, useColor);
        break;

      case 'check':
        await checkCommand(options, useColor);
        break;

      case 'catalog':
        await catalogCommand(options, useColor);
        break;

      case 'help':
      case '--help':
      case '-h':
        console.log(HELP);
        process.exit(0);
        break;

      case 'version':
      case '--version':
      case '-V':
        console.log(`ais v${VERSION}`);
        process.exit(0);
        break;

      default:
        console.error(`Unknown command: ${command}`);
        console.log(HELP);
        process.exit(1);
    }
  } catch (err) {
    console.error('Error:', err instanceof Error ? err.message : err);
    process.exit(1);
  }
}

main();
