import type { ModelUsage as ModelUsageType } from '../types';
import { formatTokens, formatCurrency } from '../types';

interface ModelUsageTableProps {
  models: ModelUsageType[];
}

export function ModelUsageTable({ models }: ModelUsageTableProps) {
  if (models.length === 0) {
    return null;
  }

  return (
    <div className="terminal-box">
      <div className="terminal-header flex justify-between">
        <span>MODEL</span>
        <span className="flex gap-6">
          <span>TOKENS</span>
          <span>%</span>
          <span>ACTUAL</span>
          <span>VALUE</span>
        </span>
      </div>
      <div className="mt-2">
        {models.map((m) => (
          <div key={m.model} className="provider-row">
            <span className="text-text font-medium text-sm truncate max-w-[200px]" title={m.model}>
              {m.model}
            </span>
            <span className="flex gap-6 items-center">
              <span className="text-green w-20 text-right">{formatTokens(m.tokens)}</span>
              <span className="text-muted w-12 text-right">{m.percentage.toFixed(1)}%</span>
              <span className="text-green w-16 text-right">{formatCurrency(m.actual_cost)}</span>
              <span className="text-warning w-16 text-right">
                {m.reference_value != null ? formatCurrency(m.reference_value) : 'N/A'}
              </span>
            </span>
          </div>
        ))}
      </div>
      <div className="mt-2 pt-2 border-t border-border">
        <span className="text-xs text-muted">
          VALUE = equivalent paid API value. N/A = no configured reference pricing.
        </span>
      </div>
    </div>
  );
}
