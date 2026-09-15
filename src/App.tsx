import { useDashboard } from './hooks/useDashboard';
import type { ProviderFilter } from './hooks/useDashboard';
import { TitleBar } from './components/TitleBar';
import { ProviderTable } from './components/ProviderTable';
import { ModelUsageTable } from './components/ModelUsageTable';
import { ValueDisplay } from './components/ValueDisplay';
import { TimeRangeTabs } from './components/TimeRangeTabs';
import { ActivityFeed } from './components/ActivityFeed';
import { ProviderStatus } from './components/ProviderStatus';

const PROVIDER_TABS: { id: ProviderFilter; label: string; key: string }[] = [
  { id: 'all', label: 'ALL', key: 'q' },
  { id: 'opencode', label: 'OPENCODE', key: 'w' },
  { id: 'freebuff', label: 'FREEBUFF', key: 'e' },
];

function App() {
  const dashboard = useDashboard();

  if (dashboard.loading) {
    return (
      <div className="w-full h-full flex items-center justify-center bg-bg">
        <div className="text-muted text-sm font-mono">
          <span className="text-green pulse">●</span> Loading...
        </div>
      </div>
    );
  }

  return (
    <div className="w-full h-full flex flex-col bg-bg overflow-hidden">
      <TitleBar
        syncing={dashboard.syncing}
        onSync={dashboard.sync}
      />

      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        <StatsSummary summary={dashboard.data?.summary ?? null} />

        <div className="divider" />

        <div className="flex gap-1">
          {PROVIDER_TABS.map((tab) => (
            <button
              key={tab.id}
              onClick={() => dashboard.setProviderFilter(tab.id)}
              className={`tab ${dashboard.providerFilter === tab.id ? 'active' : ''}`}
            >
              <span className="text-muted">[{tab.key}]</span> {tab.label}
            </button>
          ))}
        </div>

        <ProviderTable
          providers={dashboard.data?.provider_usage ?? []}
        />

        <div className="divider" />

        <ModelUsageTable
          models={dashboard.data?.model_usage ?? []}
        />

        {dashboard.data?.model_usage && dashboard.data.model_usage.length > 0 && (
          <div className="divider" />
        )}

        <ValueDisplay summary={dashboard.data?.summary ?? null} />

        <div className="divider" />

        <TimeRangeTabs
          active={dashboard.timeRange}
          onChange={dashboard.setTimeRange}
        />

        <div className="divider" />

        <ActivityFeed entries={dashboard.data?.activity ?? []} />

        <div className="divider" />

        <ProviderStatus
          providers={dashboard.data?.providers ?? []}
        />
      </div>
    </div>
  );
}

function StatsSummary({ summary }: { summary: any }) {
  if (!summary) return null;
  return (
    <div className="grid grid-cols-4 gap-2">
      <div className="card p-3">
        <div className="text-xs text-muted mb-1">TOKENS</div>
        <div className="text-lg font-mono text-green">{summary.total_tokens.toLocaleString()}</div>
      </div>
      <div className="card p-3">
        <div className="text-xs text-muted mb-1">REQUESTS</div>
        <div className="text-lg font-mono text-green">{summary.request_count.toLocaleString()}</div>
      </div>
      <div className="card p-3">
        <div className="text-xs text-muted mb-1">ACTUAL COST</div>
        <div className="text-lg font-mono text-green">${summary.actual_cost.toFixed(4)}</div>
      </div>
      <div className="card p-3">
        <div className="text-xs text-muted mb-1">REFERENCE VALUE</div>
        <div className="text-lg font-mono text-cyan">${summary.reference_value.toFixed(4)}</div>
      </div>
    </div>
  );
}

export default App;
