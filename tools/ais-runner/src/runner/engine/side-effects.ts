import type { RunnerContext, RunnerEngineEvent, RunnerSdkModule } from '../../types.js';

export function applyRunnerSideEffects(sdk: RunnerSdkModule, ctx: RunnerContext, ev: RunnerEngineEvent): void {
  // RUN-011: fan-out workflow query results into the legacy/flat runtime.query bag
  // so protocol actions/calculated_fields that reference `query["..."]` can work.
  if (ev.type === 'query_result') {
    const queryId = ev.node?.source?.query;
    if (typeof queryId === 'string' && queryId.length > 0) {
      sdk.setQueryResult(ctx, queryId, ev.outputs ?? {});
    }
  }
}
