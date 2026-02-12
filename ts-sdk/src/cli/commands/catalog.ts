/**
 * Catalog command - export workspace catalog cards for agent search
 */

import { writeFile, stat } from 'node:fs/promises';
import { relative } from 'node:path';
import { buildCatalog, loadDirectory } from '../../index.js';
import type { CLIOptions } from '../utils.js';

export async function catalogCommand(options: CLIOptions, _useColor: boolean): Promise<void> {
  const workspaceDir = options.paths[0];
  if (!workspaceDir) {
    console.error('Missing workspace directory for catalog command');
    process.exit(1);
    return;
  }

  const st = await stat(workspaceDir).catch(() => null);
  if (!st || !st.isDirectory()) {
    console.error(`Catalog workspace must be a directory: ${workspaceDir}`);
    process.exit(1);
    return;
  }

  const dirResult = await loadDirectory(workspaceDir, { recursive: options.recursive });
  const catalog = buildCatalog(dirResult);
  const raw = JSON.stringify(catalog, null, options.pretty ? 2 : undefined);

  if (options.outPath) {
    await writeFile(options.outPath, `${raw}\n`, 'utf-8');
    if (!options.quiet) {
      console.log(JSON.stringify({ kind: 'catalog_written', out: options.outPath, hash: catalog.hash, workspace: relative(process.cwd(), workspaceDir) }, null, 2));
    }
    return;
  }

  process.stdout.write(`${raw}\n`);
}

