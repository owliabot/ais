export function formatPlanSummary(plan: any): string {
  const nodes: any[] = Array.isArray(plan?.nodes) ? plan.nodes : [];
  const lines: string[] = [];
  lines.push(`plan.schema=${String(plan?.schema ?? '')}`);
  lines.push(`plan.nodes=${nodes.length}`);
  for (const n of nodes) {
    const id = String(n?.id ?? '');
    const chain = String(n?.chain ?? '');
    const kind = String(n?.kind ?? '');
    const execType = String(n?.execution?.type ?? '');
    const deps = Array.isArray(n?.deps) ? n.deps.join(',') : '';
    lines.push(`- ${id} chain=${chain} kind=${kind} exec=${execType}${deps ? ` deps=[${deps}]` : ''}`);
  }
  return `${lines.join('\n')}\n`;
}

