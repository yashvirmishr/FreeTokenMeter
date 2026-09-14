import { useDashboard } from './hooks/useDashboard';
import { TitleBar } from './components/TitleBar';
import { StatsOverview } from './components/StatsOverview';
import { ProviderTable } from './components/ProviderTable';
import { ModelUsageTable } from './components/ModelUsageTable';
import { ValueDisplay } from './components/ValueDisplay';
import { TimeRangeTabs } from './components/TimeRangeTabs';
import { ActivityFeed } from './components/ActivityFeed';
import { ProviderStatus } from './components/ProviderStatus';

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
        <StatsOverview summary={dashboard.data?.summary ?? null} />

        <div className="divider" />

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

export default App;
