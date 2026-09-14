import type { ProviderUsage } from '../types';
import { formatTokens, formatCurrency } from '../types';

interface ProviderTableProps {
  providers: ProviderUsage[];
}

export function ProviderTable({ providers }: ProviderTableProps) {
  return (
    <div className="terminal-box">
      <div className="terminal-header flex justify-between">
        <span>PROVIDER</span>
        <span className="flex gap-8">
          <span>TOKENS</span>
          <span>%</span>
          <span>EST. VALUE</span>
        </span>
      </div>
      <div className="mt-2">
        {providers.length === 0 ? (
          <div className="text-muted text-xs py-2">No usage data yet</div>
        ) : (
          providers.map((p) => (
            <div key={p.provider} className="provider-row">
              <span className="text-text font-medium capitalize">{p.provider}</span>
              <span className="flex gap-8 items-center">
                <span className="text-green w-20 text-right">{formatTokens(p.tokens)}</span>
                <span className="text-muted w-12 text-right">{p.percentage.toFixed(1)}%</span>
                <span className="text-warning w-16 text-right">{formatCurrency(p.estimated_value)}</span>
              </span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
