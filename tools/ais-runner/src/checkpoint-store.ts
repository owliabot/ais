import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';

export class FileCheckpointStore {
  constructor(
    private readonly sdk: any,
    private readonly filePath: string
  ) {}

  async load(): Promise<any | null> {
    try {
      const raw = await readFile(this.filePath, 'utf-8');
      return this.sdk.deserializeCheckpoint(raw);
    } catch {
      return null;
    }
  }

  async save(checkpoint: any): Promise<void> {
    await mkdir(dirname(this.filePath), { recursive: true });
    const raw = this.sdk.serializeCheckpoint(checkpoint, { pretty: true });
    await writeFile(this.filePath, raw, 'utf-8');
  }
}

