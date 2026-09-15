import type { PricingRegistryView } from '../types';
import { formatRateDetail } from '../types';

interface PricingPanelProps {
  registry: PricingRegistryView | null;
  loading: boolean;
  error: string | null;
  onReload: () => void;
  onClose: () => void;
}

/**
 * `$ pricing` — pricing maintenance view.
 *
 * Known models come from the centralized registry; unknown models are models
 * observed in local usage history that have no pricing entry yet.
 */
export function PricingPanel({ registry, loading, error, onReload, onClose }: PricingPanelProps) {
  return (
    <div className="terminal-box fade-in">
      <div className="flex items-center justify-between">
        <span className="terminal-header border-0 p-0">$ pricing</span>
        <span className="flex items-center gap-3">
          {registry && <span className="text-xs text-cyan">{registry.version}</span>}
          <button
            onClick={onReload}
            className="text-xs text-muted hover:text-green transition-colors"
            title="Reload registry"
          >
            ↻
          </button>
          <button
            onClick={onClose}
            className="text-xs text-muted hover:text-green transition-colors"
            title="Close [p]"
          >
            [x]
          </button>
        </span>
      </div>

      {loading && !registry && (
        <div className="mt-2 text-xs text-muted">loading registry…</div>
      )}

      {error && (
        <div className="mt-2 text-xs text-error">pricing registry error: {error}</div>
      )}

      {registry && (
        <div className="mt-2 max-h-72 overflow-y-auto space-y-3">
          <section>
            <div className="pricing-section">YOUR MODELS</div>
            <div className="pricing-rule" />
            {registry.known.length === 0 ? (
              <div className="text-xs text-muted">no models with usage yet</div>
            ) : registry.known.map((p) => (
              <div key={`${p.provider}/${p.model_id}`} className="mb-3">
                <div className="flex items-baseline gap-2 text-xs">
                  <span className="text-muted">{p.provider} /</span>
                  <span className="text-text">{p.model_id}</span>
                  {p.is_free ? (
                    <span className="text-green">FREE</span>
                  ) : (
                    <span className="text-warning">PAID</span>
                  )}
                  {p.source !== 'ftm_override' && (
                    <span className="text-muted text-[10px]">({p.source})</span>
                  )}
                </div>
                <div className="text-xs text-muted pl-2 mt-0.5">
                  <span className="text-cyan">IN</span>{' '}
                  <span className="text-text">${p.actual.input_per_million.toFixed(2)}</span>
                  <span className="text-muted">/M</span>
                  <span className="text-muted mx-1.5">·</span>
                  <span className="text-green">OUT</span>{' '}
                  <span className="text-text">${p.actual.output_per_million.toFixed(2)}</span>
                  <span className="text-muted">/M</span>
                  {p.actual.cached_per_million > 0 && (
                    <>
                      <span className="text-muted mx-1.5">·</span>
                      <span className="text-warning">CACHE</span>{' '}
                      <span className="text-text">${p.actual.cached_per_million.toFixed(4)}</span>
                      <span className="text-muted">/M</span>
                    </>
                  )}
                </div>
                {p.reference_model && (
                  <div className="text-xs text-muted pl-2">
                    ref: <span className="text-cyan">{p.reference_model}</span>
                    {p.reference && ` (${formatRateDetail(p.reference)})`}
                  </div>
                )}
              </div>
            ))}
          </section>

          <section>
            <div className="pricing-section">UNPRICED MODELS</div>
            <div className="pricing-rule" />
            {registry.unknown.length === 0 ? (
              <div className="text-xs text-muted">all models have pricing — no unknowns</div>
            ) : (
              registry.unknown.map((m) => (
                <div key={`${m.provider}/${m.model}`} className="mb-2">
                  <div className="flex items-baseline gap-2 text-xs">
                    <span className="text-muted">{m.provider} /</span>
                    <span className="text-text">{m.model}</span>
                    <span className="text-warning ml-auto">
                      {m.tokens.toLocaleString('en-US')} tokens
                    </span>
                  </div>
                  <div className="text-xs text-muted pl-2">
                    {m.records} record(s) · no pricing entry
                  </div>
                </div>
              ))
            )}
          </section>

          <div className="pt-2 border-t border-border text-xs text-muted">
            IN/OUT/CACHE = price per million tokens. Unpriced models are reported as unpriced, never as $0.
          </div>
        </div>
      )}
    </div>
  );
}
