/**
 * File loader - load AIS documents from filesystem
 */
import { readFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';
import { parseAIS, parseProtocolSpec, parsePack, parseWorkflow } from './parser.js';
import { createContext, registerProtocol } from './resolver/index.js';
import type { ResolverContext } from './resolver/index.js';
import type { AnyAISDocument, ProtocolSpec, Pack, Workflow } from './schema/index.js';

export interface LoadResult<T> {
  path: string;
  document: T;
}

export interface LoadError {
  path: string;
  error: string;
}

export interface DirectoryLoadResult {
  protocols: LoadResult<ProtocolSpec>[];
  packs: LoadResult<Pack>[];
  workflows: LoadResult<Workflow>[];
  errors: LoadError[];
}

/**
 * Load a single AIS document from file
 */
export async function loadFile(filePath: string): Promise<AnyAISDocument> {
  const content = await readFile(filePath, 'utf-8');
  return parseAIS(content, { source: filePath });
}

/**
 * Load a Protocol Spec from file
 */
export async function loadProtocol(filePath: string): Promise<ProtocolSpec> {
  const content = await readFile(filePath, 'utf-8');
  return parseProtocolSpec(content, { source: filePath });
}

/**
 * Load a Pack from file
 */
export async function loadPack(filePath: string): Promise<Pack> {
  const content = await readFile(filePath, 'utf-8');
  return parsePack(content, { source: filePath });
}

/**
 * Load a Workflow from file
 */
export async function loadWorkflow(filePath: string): Promise<Workflow> {
  const content = await readFile(filePath, 'utf-8');
  return parseWorkflow(content, { source: filePath });
}

/**
 * Determine expected AIS document type from filename
 */
function getExpectedTypeFromFilename(
  filename: string
): 'ais/1.0' | 'ais-pack/1.0' | 'ais-flow/1.0' | null {
  if (filename.endsWith('.ais-pack.yaml') || filename.endsWith('.ais-pack.yml')) {
    return 'ais-pack/1.0';
  }
  if (filename.endsWith('.ais-flow.yaml') || filename.endsWith('.ais-flow.yml')) {
    return 'ais-flow/1.0';
  }
  if (filename.endsWith('.ais.yaml') || filename.endsWith('.ais.yml')) {
    return 'ais/1.0';
  }
  return null;
}

/**
 * Check if a file is an AIS document
 */
function isAISFile(filename: string): boolean {
  return getExpectedTypeFromFilename(filename) !== null;
}

/**
 * Load all AIS documents from a directory
 */
export async function loadDirectory(
  dirPath: string,
  options: { recursive?: boolean } = {}
): Promise<DirectoryLoadResult> {
  const { recursive = true } = options;
  const result: DirectoryLoadResult = {
    protocols: [],
    packs: [],
    workflows: [],
    errors: [],
  };

  async function processDirectory(currentPath: string): Promise<void> {
    const entries = await readdir(currentPath);

    for (const entry of entries) {
      const fullPath = join(currentPath, entry);
      const stats = await stat(fullPath);

      if (stats.isDirectory()) {
        if (recursive) {
          await processDirectory(fullPath);
        }
        continue;
      }

      if (!stats.isFile() || !isAISFile(entry)) {
        continue;
      }

      try {
        const content = await readFile(fullPath, 'utf-8');
        const doc = parseAIS(content, { source: fullPath });

        switch (doc.schema) {
          case 'ais/1.0':
            result.protocols.push({ path: fullPath, document: doc });
            break;
          case 'ais-pack/1.0':
            result.packs.push({ path: fullPath, document: doc });
            break;
          case 'ais-flow/1.0':
            result.workflows.push({ path: fullPath, document: doc });
            break;
        }
      } catch (err) {
        result.errors.push({
          path: fullPath,
          error: err instanceof Error ? err.message : String(err),
        });
      }
    }
  }

  await processDirectory(dirPath);
  return result;
}

/**
 * Load directory and create a ResolverContext with all protocols registered
 */
export async function loadDirectoryAsContext(
  dirPath: string,
  options: { recursive?: boolean } = {}
): Promise<{ context: ResolverContext; result: DirectoryLoadResult }> {
  const result = await loadDirectory(dirPath, options);
  const context = createContext();

  for (const { document } of result.protocols) {
    registerProtocol(context, document);
  }

  return { context, result };
}
