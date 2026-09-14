import type { ProviderUsage as ProviderUsageType } from '../types';
import { formatTokens, formatCurrency } from '../types';

interface ProviderTableProps {
  providers: ProviderUsageType[];
}

export function ProviderTable({ providers }: ProviderTableProps) {
  if (providers.length === 0) {
    return (
      <div className="terminal-box">
        <div className="terminal-header">PROVIDER</div>
        <div className="mt-2 text-muted text-sm">No usage data yet</div>
      </div>
    );
  }

  return (
    <div className="terminal-box">
      <div className="terminal-header flex justify-between">
        <span>PROVIDER</span>
        <span className="flex gap-8">
          <span>TOKENS</span>
          <span>%</span>
          <span>ACTUAL</span>
        </span>
      </div>
      <div className="mt-2">
        {providers.map((p) => (
          <div key={p.provider} className="provider-row">
            <span className="text-text font-medium text-sm">{p.provider}</span>
            <span className="flex gap-8 items-center">
              <span className="text-green w-20 text-right">{formatTokens(p.tokens)}</span>
              <span className="text-muted w-12 text-right">{p.percentage.toFixed(1)}%</span>
              <span className="text-green w-16 text-right">{formatCurrency(p.actual_cost)}</span>
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
