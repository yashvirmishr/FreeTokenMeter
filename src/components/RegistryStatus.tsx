import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { RegistryStatusView } from '../types';

export function RegistryStatus() {
  const [status, setStatus] = useState<RegistryStatusView | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    invoke<RegistryStatusView>('get_registry_status').then(setStatus).catch(() => {});
  }, []);

  const handleRefresh = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const updated = await invoke<RegistryStatusView>('refresh_pricing');
      setStatus(updated);
    } catch {
      // ignore refresh failures
    } finally {
      setRefreshing(false);
    }
  };

  if (!status) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-muted">
        <span className="w-1.5 h-1.5 rounded-full bg-muted" />
        PRICING DB
      </div>
    );
  }

  const isCurrent = !status.needs_refresh;
  const dotColor = isCurrent ? 'bg-green' : 'bg-warning';
  const label = isCurrent ? 'CURRENT' : 'STALE';
  const ageLabel = status.cache_age ? ` (${status.cache_age})` : '';

  return (
    <button
      onClick={handleRefresh}
      disabled={refreshing}
      className="flex items-center gap-1.5 text-xs text-muted hover:text-green transition-colors cursor-pointer disabled:opacity-50"
      title={`LiteLLM registry: ${status.model_count} models${ageLabel}. Click to refresh.`}
    >
      <span className={`w-1.5 h-1.5 rounded-full ${refreshing ? 'bg-muted animate-pulse' : dotColor}`} />
      PRICING DB <span className="text-green">{label}</span>
      {status.model_count > 0 && (
        <span className="text-muted">({status.model_count.toLocaleString()})</span>
      )}
    </button>
  );
}
