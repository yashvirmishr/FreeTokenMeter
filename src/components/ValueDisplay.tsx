import type { UsageSummary } from '../types';
import { formatCurrency } from '../types';

interface ValueDisplayProps {
  summary: UsageSummary | null;
}

/**
 * The three value figures, kept strictly separate:
 *
 *   ACTUAL COST      what the user is billed — N/A when no per-token spend exists
 *   REFERENCE VALUE  what this usage would cost at list rates
 *   FREE VALUE       reference − actual; only shown when both sides are known
 *
 * Under a subscription there is no "saved $X" claim, because the alternative
 * expenditure is not known.
 */
export function ValueDisplay({ summary }: ValueDisplayProps) {
  const s = summary;
  const nothingPriced = s != null && s.request_count > 0 && s.unpriced_records >= s.request_count;
  const credits = s?.billing_units ?? 0;

  const actualKnown = !nothingPriced && s != null && s.unbilled_records < s.request_count;
  const referenceKnown = !nothingPriced && s != null && s.reference_value > 0;

  return (
    <div className="terminal-box">
      <div className="terminal-header mb-2">VALUE</div>
      <div className="space-y-1.5">
        <div className="flex justify-between items-baseline">
          <span className="stat-label">ACTUAL COST</span>
          <span className={`text-lg font-bold ${actualKnown ? 'text-green' : 'text-muted'}`}>
            {actualKnown ? formatCurrency(s?.actual_cost ?? 0) : 'N/A'}
          </span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">REFERENCE API VALUE</span>
          <span className={`text-lg font-bold ${referenceKnown ? 'text-warning' : 'text-muted'}`}>
            {referenceKnown ? formatCurrency(s?.reference_value ?? 0) : 'N/A'}
          </span>
        </div>
        <div className="flex justify-between items-baseline">
          <span className="stat-label">FREE VALUE</span>
          <span className={`text-lg font-bold ${referenceKnown && actualKnown ? 'text-green' : 'text-muted'}`}>
            {referenceKnown && actualKnown ? formatCurrency(s?.free_value ?? 0) : 'N/A'}
          </span>
        </div>
        {credits > 0 && (
          <div className="flex justify-between items-baseline">
            <span className="stat-label">BILLED UNITS</span>
            <span className="text-lg font-bold text-cyan">{credits.toFixed(0)} cr</span>
          </div>
        )}
      </div>
      <div className="mt-2 pt-2 border-t border-border">
        <span className="text-xs text-muted">
          ACTUAL N/A = subscription/entitlement usage with no per-token spend. REFERENCE
          is the equivalent paid API value. FREE VALUE needs both, so a flat-fee plan is
          never reported as money saved.
        </span>
        {(s?.unbilled_records ?? 0) > 0 && (
          <div className="text-xs text-muted mt-1">
            {s?.unbilled_records} record(s) reported no monetary cost.
          </div>
        )}
        {(s?.unpriced_records ?? 0) > 0 && (
          <div className="text-xs text-warning mt-1">
            {s?.unpriced_records} unpriced record(s) ({s?.unpriced_tokens.toLocaleString()} tokens)
            excluded — PRICING: UNKNOWN. Press [p] for the pricing registry.
          </div>
        )}
      </div>
    </div>
  );
}
