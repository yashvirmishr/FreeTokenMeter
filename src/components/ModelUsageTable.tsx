import type { ModelUsage as ModelUsageType, ProviderStatus } from '../types';
import { formatBilling, formatCurrency, formatTokens, isUnpriced } from '../types';

interface ModelUsageTableProps {
  models: ModelUsageType[];
  status: ProviderStatus[];
}

const SOURCE_COLORS: Record<string, string> = {
  ftm_override: 'bg-green',
  litellm: 'bg-cyan',
  litellm_cached: 'bg-cyan',
  provider_reported: 'bg-warning',
  unknown: 'bg-red',
};

const SOURCE_LABELS: Record<string, string> = {
  ftm_override: 'FTM',
  litellm: 'LITELLM',
  litellm_cached: 'LITELLM',
  provider_reported: 'PROVIDER',
  unknown: 'UNKNOWN',
};

function PricingSourceDot({ source }: { source: string | null }) {
  if (!source) return null;
  const color = SOURCE_COLORS[source] ?? 'bg-muted';
  const label = SOURCE_LABELS[source] ?? source.toUpperCase();
  return (
    <span
      className={`inline-block w-1.5 h-1.5 rounded-full ${color} ml-1.5 flex-shrink-0`}
      title={label}
    />
  );
}

/**
 * `PROVIDER  MODEL  TOKENS  ACTUAL / BILLING  REFERENCE  STATUS`
 *
 * The billing column is provider-aware: a subscription provider shows `N/A`, a
 * credit-metered provider shows its units, and a promotional provider shows
 * `$0.00` — which is a price, not a guess.
 */
export function ModelUsageTable({ models, status }: ModelUsageTableProps) {
  if (models.length === 0) {
    return null;
  }

  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();

  return (
    <div className="terminal-box">
      <div className="terminal-header flex justify-between">
        <span>MODEL</span>
        <span className="flex gap-4 sm:gap-6">
          <span>TOKENS</span>
          <span className="hidden sm:inline">%</span>
          <span>ACTUAL / BILLING</span>
          <span className="hidden sm:inline">REFERENCE</span>
        </span>
      </div>

      <div className="mt-2 overflow-x-auto">
        {models.map((m) => {
          const unpriced = isUnpriced(m.pricing_status);
          const billing = formatBilling(m.billing_units, m.billing_unit);
          return (
            <div key={`${m.provider}:${m.model}`} className="provider-row min-w-[340px]">
              <span className="flex items-center gap-2 min-w-0">
                <span className="text-cyan text-xs whitespace-nowrap">{label(m.provider)}</span>
                <span
                  className="text-text font-medium text-sm truncate max-w-[120px] sm:max-w-[180px]"
                  title={m.model}
                >
                  {m.model}
                </span>
                {unpriced && (
                  <span
                    className="text-warning text-xs whitespace-nowrap"
                    title="Unknown model — no pricing entry. Never assumed free."
                  >
                    UNKNOWN
                  </span>
                )}
                <PricingSourceDot source={m.pricing_source} />
              </span>
              <span className="flex gap-4 sm:gap-6 items-center">
                <span className="text-green w-20 text-right">{formatTokens(m.tokens)}</span>
                <span className="text-muted w-12 text-right hidden sm:inline">
                  {m.percentage.toFixed(1)}%
                </span>
                <span
                  className={`w-20 text-right ${
                    m.actual_cost == null && billing === '—' ? 'text-muted' : 'text-green'
                  }`}
                  title={m.billing_metric_name ?? undefined}
                >
                  {m.actual_cost == null ? billing : formatCurrency(m.actual_cost)}
                </span>
                <span className="text-warning w-16 text-right hidden sm:inline">
                  {formatCurrency(m.reference_value)}
                </span>
              </span>
            </div>
          );
        })}
      </div>

      <div className="mt-2 pt-2 border-t border-border">
        <span className="text-xs text-muted">
          REFERENCE = equivalent paid API value. N/A = no reference pricing configured.
          UNKNOWN = the model is missing from the pricing registry, so no cost is claimed.
        </span>
      </div>
    </div>
  );
}
