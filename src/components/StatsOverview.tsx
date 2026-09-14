import type { UsageSummary } from '../types';
import { formatTokens } from '../types';

interface StatsOverviewProps {
  summary: UsageSummary | null;
}

export function StatsOverview({ summary }: StatsOverviewProps) {
  const s = summary ?? {
    total_tokens: 0,
    input_tokens: 0,
    output_tokens: 0,
    reasoning_tokens: 0,
    cached_tokens: 0,
    request_count: 0,
    estimated_api_value: 0,
    actual_cost: 0,
    free_value: 0,
  };

  return (
    <div className="terminal-box">
      <div className="text-xs text-muted mb-3 font-mono">
        $ freetokenmeter --watch
      </div>
      <div className="grid grid-cols-2 gap-x-8 gap-y-2">
        <div className="flex justify-between items-baseline">
          <span className="stat-label">TOKENS</span>
          <span className="stat-value">{formatTokens(s.total_tokens)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">REQUESTS</span>
          <span className="stat-value">{s.request_count.toLocaleString()}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">INPUT</span>
          <span className="stat-value">{formatTokens(s.input_tokens)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">OUTPUT</span>
          <span className="stat-value">{formatTokens(s.output_tokens)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">REASONING</span>
          <span className="stat-value">{formatTokens(s.reasoning_tokens)}</span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">CACHED</span>
          <span className="stat-value">{formatTokens(s.cached_tokens)}</span>
        </div>
      </div>
    </div>
  );
}
