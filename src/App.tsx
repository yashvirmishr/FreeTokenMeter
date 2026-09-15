import { useEffect, useMemo, useState } from 'react';
import { ALL_PROVIDERS, useDashboard } from './hooks/useDashboard';
import { useDesktop } from './hooks/useDesktop';
import { usePricingRegistry } from './hooks/usePricingRegistry';
import { TitleBar } from './components/TitleBar';
import { ProviderTable } from './components/ProviderTable';
import { ModelUsageTable } from './components/ModelUsageTable';
import { ValueDisplay } from './components/ValueDisplay';
import { TimeRangeTabs } from './components/TimeRangeTabs';
import { ActivityFeed } from './components/ActivityFeed';
import { ProviderStatusPanel } from './components/ProviderStatus';
import { SettingsPanel } from './components/SettingsPanel';
import { PricingPanel } from './components/PricingPanel';
import { SyncBanner } from './components/SyncBanner';
import type { UsageSummary } from './types';
import { formatCurrency } from './types';

/** Matches `sync::BACKGROUND_SYNC_INTERVAL_SECS` in the Rust backend. */
const SYNC_INTERVAL_SECS = 30;

function App() {
  const dashboard = useDashboard();
  const desktop = useDesktop();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pricingOpen, setPricingOpen] = useState(false);
  const pricing = usePricingRegistry(pricingOpen);

  const { sync } = dashboard;
  const { toggleAlwaysOnTop, hideToTray, quit } = desktop;

  const providers = dashboard.data?.providers ?? [];
  const status = providers;

  /**
   * Provider tabs are derived from the providers the backend reports, so adding
   * a provider needs no frontend change. `[q]` clears the filter; the rest are
   * click-only to avoid colliding with the numeric time-range keys.
   */
  const tabs = useMemo(
    () => [
      { id: ALL_PROVIDERS, label: 'ALL', key: 'q' },
      ...providers.map((provider) => ({
        id: provider.id,
        label: provider.short_label,
        key: '',
      })),
    ],
    [providers],
  );

  // Keyboard shortcuts. Panels capture their keys first so `1` can mean
  // "toggle always on top" inside settings while `1` still means "today"
  // on the dashboard.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const key = e.key.toLowerCase();

      if (key === 'escape') {
        setSettingsOpen(false);
        setPricingOpen(false);
        return;
      }

      if (settingsOpen) {
        if (key === '1') toggleAlwaysOnTop();
        else if (key === 'h') hideToTray();
        else if (key === 'x') quit();
        else if (key === 's') setSettingsOpen(false);
        return;
      }

      if (pricingOpen) {
        if (key === 'p') setPricingOpen(false);
        return;
      }

      switch (key) {
        case 'r':
          sync();
          break;
        case 's':
          setSettingsOpen(true);
          break;
        case 'p':
          setPricingOpen(true);
          break;
        case 'q':
          dashboard.setProviderFilter(ALL_PROVIDERS);
          break;
      }
    };

    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [settingsOpen, pricingOpen, sync, dashboard, toggleAlwaysOnTop, hideToTray, quit]);

  if (dashboard.loading && !dashboard.data) {
    return (
      <div className="w-full h-full flex items-center justify-center bg-bg">
        <div className="text-muted text-sm font-mono">
          <span className="text-green pulse">●</span> Loading...
        </div>
      </div>
    );
  }

  const panelOpen = settingsOpen || pricingOpen;

  return (
    <div className="w-full h-full flex flex-col bg-bg overflow-hidden">
      <TitleBar
        syncing={dashboard.syncing}
        alwaysOnTop={desktop.windowState?.always_on_top ?? false}
        settingsOpen={settingsOpen}
        pricingOpen={pricingOpen}
        onSync={sync}
        onToggleSettings={() => {
          setPricingOpen(false);
          setSettingsOpen((open) => !open);
        }}
        onTogglePricing={() => {
          setSettingsOpen(false);
          setPricingOpen((open) => !open);
        }}
        onHide={hideToTray}
      />

      <div className="flex-1 overflow-y-auto p-3 sm:p-4 space-y-3">
        {settingsOpen && (
          <SettingsPanel
            windowState={desktop.windowState}
            prefs={desktop.prefs}
            providers={providers}
            futureProviders={desktop.futureProviders}
            pricingVersion={pricing.registry?.version ?? null}
            registryModels={pricing.registry?.registry_models ?? 0}
            syncIntervalSecs={SYNC_INTERVAL_SECS}
            onToggleAlwaysOnTop={toggleAlwaysOnTop}
            onToggleProvider={async (id, enabled) => {
              await desktop.setProviderEnabled(id, enabled);
              await dashboard.refresh();
            }}
            onSetBackfillDays={desktop.setBackfillDays}
            onHide={hideToTray}
            onQuit={quit}
            onClose={() => setSettingsOpen(false)}
          />
        )}

        {pricingOpen && (
          <PricingPanel
            registry={pricing.registry}
            loading={pricing.loading}
            error={pricing.error}
            onReload={pricing.reload}
            onClose={() => setPricingOpen(false)}
          />
        )}

        {desktop.error && (
          <div className="terminal-box text-xs text-error">{desktop.error}</div>
        )}

        <SyncBanner
          report={dashboard.lastSync}
          hiddenNotice={desktop.hiddenNotice}
          onDismissHidden={desktop.dismissHiddenNotice}
        />

        <StatsSummary summary={dashboard.data?.summary ?? null} />

        <div className="divider" />

        <div className="flex gap-1 flex-wrap">
          {tabs.map((tab) => (
            <button
              key={tab.id}
              onClick={() => dashboard.setProviderFilter(tab.id)}
              className={`tab ${dashboard.providerFilter === tab.id ? 'active' : ''}`}
            >
              {tab.key ? <span className="text-muted">[{tab.key}]</span> : null} {tab.label}
            </button>
          ))}
        </div>

        <ProviderTable providers={dashboard.data?.provider_usage ?? []} status={status} />

        <div className="divider" />

        <ModelUsageTable models={dashboard.data?.model_usage ?? []} status={status} />

        {dashboard.data?.model_usage && dashboard.data.model_usage.length > 0 && (
          <div className="divider" />
        )}

        <ValueDisplay summary={dashboard.data?.summary ?? null} />

        <div className="divider" />

        <TimeRangeTabs
          active={dashboard.timeRange}
          onChange={dashboard.setTimeRange}
          disabled={panelOpen}
        />

        <div className="divider" />

        <ActivityFeed entries={dashboard.data?.activity ?? []} status={status} />

        <div className="divider" />

        <ProviderStatusPanel providers={providers} />
      </div>
    </div>
  );
}

function StatsSummary({ summary }: { summary: UsageSummary | null }) {
  if (!summary) return null;

  const unpriced = summary.unpriced_records ?? 0;
  // Only when *every* record is unpriced do we have no cost to report at all.
  const nothingPriced = summary.request_count > 0 && unpriced >= summary.request_count;
  const marker = unpriced > 0 ? '*' : '';
  const actualKnown = !nothingPriced && summary.unbilled_records < summary.request_count;
  const referenceKnown = !nothingPriced && summary.reference_value > 0;

  return (
    <div className="space-y-2">
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-2">
        <div className="card p-2 sm:p-3">
          <div className="text-xs text-muted mb-1">TOKENS</div>
          <div className="text-lg font-mono text-green">
            {summary.total_tokens.toLocaleString()}
          </div>
        </div>
        <div className="card p-2 sm:p-3">
          <div className="text-xs text-muted mb-1">REQUESTS</div>
          <div className="text-lg font-mono text-green">
            {summary.request_count.toLocaleString()}
          </div>
        </div>
        <div className="card p-2 sm:p-3">
          <div className="text-xs text-muted mb-1">ACTUAL COST</div>
          <div className={`text-lg font-mono ${actualKnown ? 'text-green' : 'text-muted'}`}>
            {actualKnown ? `${formatCurrency(summary.actual_cost)}${marker}` : 'N/A'}
          </div>
        </div>
        <div className="card p-2 sm:p-3">
          <div className="text-xs text-muted mb-1">REFERENCE VALUE</div>
          <div className={`text-lg font-mono ${referenceKnown ? 'text-cyan' : 'text-muted'}`}>
            {referenceKnown ? `${formatCurrency(summary.reference_value)}${marker}` : 'N/A'}
          </div>
        </div>
        <div className="card p-2 sm:p-3">
          <div className="text-xs text-muted mb-1">BILLED UNITS</div>
          <div className="text-lg font-mono text-cyan">
            {summary.billing_units > 0 ? `${summary.billing_units.toFixed(0)} cr` : '—'}
          </div>
        </div>
      </div>

      {summary.unbilled_records > 0 && (
        <div className="terminal-box text-xs flex items-start gap-2">
          <span className="text-cyan">SUBSCRIPTION</span>
          <span className="text-muted flex-1">
            {summary.unbilled_records} of {summary.request_count} record(s) reported no
            monetary cost, so ACTUAL COST covers only the priced usage.
          </span>
        </div>
      )}

      {unpriced > 0 && (
        <div className="terminal-box text-xs flex items-start gap-2">
          <span className="text-warning">PRICING: UNKNOWN</span>
          <span className="text-muted flex-1">
            {unpriced} of {summary.request_count} record(s) ({' '}
            {summary.unpriced_tokens.toLocaleString()} tokens) use a model that is not in
            the pricing registry. They are excluded from the totals above instead of being
            counted as $0 — open [p] pricing for details.
          </span>
        </div>
      )}
    </div>
  );
}

export default App;
