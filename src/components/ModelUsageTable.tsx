import type { ModelUsage as ModelUsageType } from '../types';
import { formatTokens, formatCurrency } from '../types';

interface ModelUsageTableProps {
  models: ModelUsageType[];
}

export function ModelUsageTable({ models }: ModelUsageTableProps) {
  if (models.length === 0) {
    return null;
  }

  // Group models by provider prefix (e.g., "openai/gpt-4" → "openai")
  const getModelProvider = (model: string): string => {
    const slashIndex = model.indexOf('/');
    if (slashIndex > 0) {
      return model.substring(0, slashIndex);
    }
    // Check for known OpenCode-specific model patterns
    if (model.includes('mimo-v2.5')) return 'opencode';
    if (model.includes('big-pickle')) return 'opencode';
    if (model.includes('nemotron')) return 'opencode';
    return 'unknown';
  };

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
        {models.map((m) => {
          const provider = getModelProvider(m.model);
          return (
            <div key={m.model} className="provider-row">
              <span className="flex items-center gap-2">
                <span className="text-muted text-xs">{provider}</span>
                <span className="text-text font-medium text-sm truncate max-w-[200px]" title={m.model}>
                  {m.model}
                </span>
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
          );
        })}
      </div>
      <div className="mt-2 pt-2 border-t border-border">
        <span className="text-xs text-muted">
          VALUE = equivalent paid API value. N/A = no configured reference pricing.
        </span>
      </div>
    </div>
  );
}
