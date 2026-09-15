import type { ProviderStatus as ProviderStatusType } from '../types';
import { formatTokens, paymentModeLabel, providerGlyph, providerGlyphClass } from '../types';

interface ProviderStatusProps {
  providers: ProviderStatusType[];
}

/**
 * `$ providers health`
 *
 * Every provider is listed with its real detection state. A provider that is
 * installed but exposes no readable source is shown as USAGE UNAVAILABLE with
 * the reason — never as `0 tokens`.
 */
export function ProviderStatusPanel({ providers }: ProviderStatusProps) {
  return (
    <div className="terminal-box">
      <div className="terminal-header">PROVIDERS HEALTH</div>
      <div className="mt-2 space-y-3">
        {providers.length === 0 && (
          <div className="text-muted text-xs">No providers registered</div>
        )}
        {providers.map((p) => (
          <div key={p.id}>
            <div className="flex items-center gap-2 mb-1 flex-wrap">
              <span className={`text-sm ${providerGlyphClass(p.state)}`} aria-hidden>
                {providerGlyph(p.state)}
              </span>
              <span className="text-text font-medium text-sm">{p.name}</span>
              <span className={`text-xs ${providerGlyphClass(p.state)}`}>{p.status}</span>
              <span className="text-muted text-xs">{paymentModeLabel(p.payment_mode)}</span>
              {p.billing_metric_name && (
                <span className="text-muted text-xs">metered in {p.billing_metric_name}</span>
              )}
              {!p.enabled && <span className="text-warning text-xs">DISABLED IN SETTINGS</span>}
            </div>
            {p.detail && <div className="text-muted text-xs ml-5">{p.detail}</div>}
            <div className="text-muted text-xs ml-5 flex flex-wrap gap-x-4">
              <span>
                source: {p.source_type}
                {p.source_version ? ` v${p.source_version}` : ''}
              </span>
              {p.records > 0 && <span>records: {p.records.toLocaleString()}</span>}
              {p.tokens > 0 && <span>tokens: {formatTokens(p.tokens)}</span>}
              <span>last sync: {p.last_sync ?? 'never'}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
