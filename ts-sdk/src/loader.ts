/**
 * File loader - load AIS documents from filesystem
 */
import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { AISParseError, parseAIS, parseProtocolSpec, parsePack, parseWorkflow } from './parser.js';
import { createContext, parseProtocolRef, registerProtocol } from './resolver/index.js';
import type { ResolverContext } from './resolver/index.js';
import type { AnyAISDocument, ProtocolSpec, Pack, Workflow } from './schema/index.js';
import type { ExecutionTypeRegistry } from './plugins/index.js';
import { validateWorkflow } from './validator/workflow.js';
import type { WorkflowValidationResult } from './validator/workflow.js';

export interface LoadResult<T> {
  path: string;
  document: T;
}

export interface LoadError {
  path: string;
  error: string;
  kind?: 'yaml' | 'schema' | 'plugin_unknown_execution' | 'plugin_schema' | 'unknown';
  issues?: Array<{ path: string; message: string }>;
  execution_type?: string;
  field_path?: string;
  details?: unknown;
}

export interface DirectoryLoadResult {
  protocols: LoadResult<ProtocolSpec>[];
  packs: LoadResult<Pack>[];
  workflows: LoadResult<Workflow>[];
  errors: LoadError[];
}

export interface LoadWorkflowBundleOptions {
  execution_registry?: ExecutionTypeRegistry;
  strict_imports?: boolean;
  validate?: boolean;
  builtin_protocols?: ProtocolSpec[];
  pre_registered_protocols?: ProtocolSpec[];
  search_paths?: string[];
}

export interface WorkflowBundleLoadResult {
  workflow: Workflow;
  context: ResolverContext;
  imports: LoadResult<ProtocolSpec>[];
  validation?: WorkflowValidationResult;
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
 * Load a workflow with explicit protocol imports into a fresh resolver context.
 *
 * Resolution order:
 * 1) builtin_protocols (source: builtin)
 * 2) pre_registered_protocols (source: manual)
 * 3) workflow.imports.protocols (source: import)
 */
export async function loadWorkflowBundle(
  workflowPath: string,
  options: LoadWorkflowBundleOptions = {}
): Promise<WorkflowBundleLoadResult> {
  const workflow = await loadWorkflow(workflowPath);
  const context = createContext();
  const imported: LoadResult<ProtocolSpec>[] = [];

  for (const spec of options.builtin_protocols ?? []) {
    registerProtocol(context, spec, { source: 'builtin' });
  }
  for (const spec of options.pre_registered_protocols ?? []) {
    registerProtocol(context, spec, { source: 'manual' });
  }

  const protocolImports = (workflow as any).imports?.protocols;
  if (Array.isArray(protocolImports)) {
    for (const entry of protocolImports) {
      const protoRef = String(entry?.protocol ?? '');
      const importPath = String(entry?.path ?? '');
      const resolvedPath = await resolveImportedProtocolPath(workflowPath, importPath, options.search_paths ?? []);
      if (!resolvedPath) {
        throw new Error(`Workflow import path not found for ${protoRef}: ${importPath}`);
      }
      const spec = await loadProtocol(resolvedPath);
      const parsed = parseProtocolRef(protoRef);
      if (spec.meta.protocol !== parsed.protocol || (parsed.version && spec.meta.version !== parsed.version)) {
        throw new Error(
          `Workflow import mismatch for ${protoRef}: loaded ${spec.meta.protocol}@${spec.meta.version} from ${resolvedPath}`
        );
      }
      registerProtocol(context, spec, { source: 'import' });
      imported.push({ path: resolvedPath, document: spec });
    }
  }

  const shouldValidate = options.validate ?? true;
  if (!shouldValidate) {
    return { workflow, context, imports: imported };
  }
  const validation = validateWorkflow(workflow, context, { enforce_imports: options.strict_imports ?? true });
  if (!validation.valid) {
    throw new Error(`Workflow bundle validation failed: ${JSON.stringify(validation.issues)}`);
  }

  return { workflow, context, imports: imported, validation };
}

/**
 * Determine expected AIS document type from filename
 */
function getExpectedTypeFromFilename(
  filename: string
): 'ais/0.0.2' | 'ais-pack/0.0.2' | 'ais-flow/0.0.3' | null {
  if (filename.endsWith('.ais-pack.yaml') || filename.endsWith('.ais-pack.yml')) {
    return 'ais-pack/0.0.2';
  }
  if (filename.endsWith('.ais-flow.yaml') || filename.endsWith('.ais-flow.yml')) {
    return 'ais-flow/0.0.3';
  }
  if (filename.endsWith('.ais.yaml') || filename.endsWith('.ais.yml')) {
    return 'ais/0.0.2';
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
  options: {
    recursive?: boolean;
    execution_registry?: ExecutionTypeRegistry;
    /**
     * Optional ignore predicate. When it returns true, the file/dir is skipped.
     * Useful for excluding build artifacts, vendor folders, etc.
     */
    ignore?: (fullPath: string) => boolean;
  } = {}
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
      if (options.ignore?.(fullPath)) continue;
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
        const doc = parseAIS(content, { source: fullPath, execution_registry: options.execution_registry });

        switch (doc.schema) {
          case 'ais/0.0.2':
            result.protocols.push({ path: fullPath, document: doc });
            break;
          case 'ais-pack/0.0.2':
            result.packs.push({ path: fullPath, document: doc });
            break;
          case 'ais-flow/0.0.3':
            result.workflows.push({ path: fullPath, document: doc });
            break;
        }
      } catch (err) {
        result.errors.push(toLoadError(fullPath, err));
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
  options: {
    recursive?: boolean;
    execution_registry?: ExecutionTypeRegistry;
    ignore?: (fullPath: string) => boolean;
  } = {}
): Promise<{ context: ResolverContext; result: DirectoryLoadResult }> {
  const result = await loadDirectory(dirPath, options);
  const context = createContext();

  for (const { document } of result.protocols) {
    registerProtocol(context, document, { source: 'workspace' });
  }

  return { context, result };
}

function toLoadError(path: string, err: unknown): LoadError {
  if (err instanceof AISParseError) {
    const details = err.details;

    // Zod issues array
    if (Array.isArray(details) && details.every((d) => d && typeof d === 'object' && 'path' in d && 'message' in d)) {
      const issues = (details as any[]).map((i) => ({
        path: Array.isArray(i.path) ? i.path.join('.') : String(i.path),
        message: String(i.message),
      }));
      return { path, error: err.message, kind: err.message.includes('Invalid YAML') ? 'yaml' : 'schema', issues, details };
    }

    // Plugin errors from execution type validation
    if (details && typeof details === 'object' && 'type' in details) {
      const d = details as any;
      const type = typeof d.type === 'string' ? d.type : undefined;
      const fieldPath = typeof d.path === 'string' ? d.path : undefined;
      const kind =
        err.message.includes('Unknown execution type') ? 'plugin_unknown_execution' : 'plugin_schema';
      return { path, error: err.message, kind, execution_type: type, field_path: fieldPath, details };
    }

    return { path, error: err.message, kind: err.message.includes('Invalid YAML') ? 'yaml' : 'unknown', details };
  }

  return { path, error: err instanceof Error ? err.message : String(err), kind: 'unknown' };
}

async function resolveImportedProtocolPath(
  workflowPath: string,
  importPath: string,
  searchPaths: string[]
): Promise<string | null> {
  const candidates = new Set<string>();
  if (isAbsolute(importPath)) {
    candidates.add(importPath);
  } else {
    candidates.add(resolve(dirname(workflowPath), importPath));
    for (const base of searchPaths) {
      candidates.add(resolve(base, importPath));
    }
    candidates.add(resolve(process.cwd(), importPath));
  }

  for (const p of candidates) {
    if (await fileExists(p)) return p;
  }
  return null;
}

async function fileExists(path: string): Promise<boolean> {
  try {
    const s = await stat(path);
    return s.isFile();
  } catch {
    return false;
  }
}
