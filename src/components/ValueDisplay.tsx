import type { UsageSummary } from '../types';
import { formatCurrency } from '../types';

interface ValueDisplayProps {
  summary: UsageSummary | null;
}

export function ValueDisplay({ summary }: ValueDisplayProps) {
  const s = summary ?? {
    actual_cost: 0,
    reference_value: 0,
    free_value: 0,
  };

  return (
    <div className="terminal-box">
      <div className="terminal-header mb-2">VALUE</div>
      <div className="space-y-1.5">
        <div className="flex justify-between items-baseline">
          <span className="stat-label">ACTUAL COST</span>
          <span className="text-green text-lg font-bold">{formatCurrency(s.actual_cost)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">REFERENCE API VALUE</span>
          <span className="text-warning text-lg font-bold">{formatCurrency(s.reference_value)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">FREE VALUE</span>
          <span className="text-green text-lg font-bold">{formatCurrency(s.free_value)}</span>
        </div>
      </div>
      <div className="mt-2 pt-2 border-t border-border">
        <span className="text-xs text-muted">
          Equivalent paid API value at the configured reference rate.
        </span>
      </div>
    </div>
  );
}
