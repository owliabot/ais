import test from 'node:test';
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

import { loadAndValidatePlanFile } from '../dist/runner/commands/run-plan.js';

function tmpFile(name) {
  return join(tmpdir(), `ais-runner-run-plan-${process.pid}-${Date.now()}-${name}`);
}

test('loadAndValidatePlanFile returns machine-readable plan_validation_error payload', async () => {
  const planPath = tmpFile('bad-plan.json');
  await writeFile(planPath, '{"schema":"ais-plan/0.0.2","nodes":"bad"}', 'utf-8');

  const sdk = {
    parseAisJson: JSON.parse,
    ExecutionPlanSchema: {
      safeParse: () => ({
        success: false,
        error: {
          issues: [
            {
              path: ['nodes'],
              message: 'Expected array, received string',
              code: 'invalid_type',
            },
          ],
        },
      }),
    },
  };

  const result = await loadAndValidatePlanFile(sdk, planPath);
  assert.equal(result.ok, false);
  assert.equal(result.error.kind, 'plan_validation_error');
  assert.equal(result.error.path, planPath);
  assert.equal(result.error.issues[0].field_path, 'nodes');
  assert.equal(result.error.issues[0].reference, 'invalid_type');
});
