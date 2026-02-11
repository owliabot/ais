import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';
import type { RunnerEngineCheckpoint, RunnerSdkModule } from './types.js';

type CheckpointCodecSdk = Pick<RunnerSdkModule, 'deserializeCheckpoint' | 'serializeCheckpoint'>;

export class FileCheckpointStore {
  constructor(
    private readonly sdk: CheckpointCodecSdk,
    private readonly filePath: string
  ) {}

  async load(): Promise<RunnerEngineCheckpoint | null> {
    try {
      const raw = await readFile(this.filePath, 'utf-8');
      return this.sdk.deserializeCheckpoint(raw) as RunnerEngineCheckpoint;
    } catch {
      return null;
    }
  }

  async save(checkpoint: RunnerEngineCheckpoint): Promise<void> {
    await mkdir(dirname(this.filePath), { recursive: true });
    const raw = this.sdk.serializeCheckpoint(checkpoint, { pretty: true });
    await writeFile(this.filePath, raw, 'utf-8');
  }
}
