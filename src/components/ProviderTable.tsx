import type { ProviderStatus, ProviderUsage as ProviderUsageType } from '../types';
import { formatBilling, formatCurrency, formatTokens } from '../types';

interface ProviderTableProps {
  providers: ProviderUsageType[];
  status: ProviderStatus[];
}

/**
 * Per-provider totals.
 *
 * ACTUAL shows the money the user is actually billed. `N/A` is not the same as
 * `$0.00`: it means no per-token spend exists (subscription usage). Providers
 * metered in AI credits show their units instead of a dollar figure.
 */
export function ProviderTable({ providers, status }: ProviderTableProps) {
  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();

  if (providers.length === 0) {
    return (
      <div className="terminal-box">
        <div className="terminal-header">PROVIDERS</div>
        <div className="mt-2 text-muted text-sm">No usage data yet</div>
      </div>
    );
  }

  return (
    <div className="terminal-box">
      <div className="terminal-header flex justify-between">
        <span>PROVIDER</span>
        <span className="flex gap-4 sm:gap-6">
          <span>TOKENS</span>
          <span className="hidden sm:inline">%</span>
          <span>ACTUAL</span>
          <span className="hidden sm:inline">REFERENCE</span>
        </span>
      </div>
      <div className="mt-2 overflow-x-auto">
        {providers.map((p) => {
          const billing = formatBilling(p.billing_units, p.billing_unit);
          return (
            <div key={p.provider} className="provider-row min-w-[300px]">
              <span className="text-text font-medium text-sm truncate" title={p.provider}>
                {label(p.provider)}
              </span>
              <span className="flex gap-4 sm:gap-6 items-center">
                <span className="text-green w-20 text-right">{formatTokens(p.tokens)}</span>
                <span className="text-muted w-12 text-right hidden sm:inline">
                  {p.percentage.toFixed(1)}%
                </span>
                <span
                  className={`w-16 text-right ${
                    p.actual_cost == null && billing === '—' ? 'text-muted' : 'text-green'
                  }`}
                  title={p.billing_metric_name ?? undefined}
                >
                  {p.actual_cost == null ? billing : formatCurrency(p.actual_cost)}
                </span>
                <span className="text-warning w-16 text-right hidden sm:inline">
                  {formatCurrency(p.reference_value)}
                </span>
              </span>
            </div>
          );
        })}
      </div>
      <div className="mt-2 pt-2 border-t border-border text-xs text-muted">
        ACTUAL N/A = no per-token spend (subscription/entitlement usage). Credits are
        billed units, not dollars.
      </div>
    </div>
  );
}
