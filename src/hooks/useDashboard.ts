import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { DashboardData, SyncReport, TimeRange } from '../types';
import { getDaysForRange } from '../types';

/**
 * Provider filter. `'all'` or a canonical provider id — the tab list is built
 * from whatever providers the backend reports, so a new provider needs no
 * frontend change.
 */
export type ProviderFilter = string;

export const ALL_PROVIDERS: ProviderFilter = 'all';

/**
 * Dashboard data + synchronization.
 *
 * The backend runs its own synchronization loop, so this hook only triggers an
 * initial sync and refreshes whenever the backend reports a completed pass
 * (`sync://complete`). Manual sync and background sync share the same path.
 */
export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>('today');
  const [providerFilter, setProviderFilter] = useState<ProviderFilter>(ALL_PROVIDERS);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [lastSync, setLastSync] = useState<SyncReport | null>(null);
  const startedRef = useRef(false);

  const fetchDashboard = useCallback(async () => {
    try {
      const days = getDaysForRange(timeRange);
      const provider = providerFilter === ALL_PROVIDERS ? null : providerFilter;
      const result = await invoke<DashboardData>('get_dashboard', { days, provider });
      setData(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [timeRange, providerFilter]);

  /** Manual sync — exactly the same backend path the tray uses. */
  const sync = useCallback(async () => {
    setSyncing(true);
    try {
      const report = await invoke<SyncReport>('sync_all');
      setLastSync(report);
      await fetchDashboard();
    } catch (err) {
      setError(String(err));
    } finally {
      setSyncing(false);
    }
  }, [fetchDashboard]);

  useEffect(() => {
    fetchDashboard();
  }, [fetchDashboard]);

  // Sync once on startup; afterwards the backend scheduler keeps history fresh.
  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;

    invoke<SyncReport | null>('get_last_sync_report')
      .then((report) => { if (report) setLastSync(report); })
      .catch(() => { /* first run may have no report yet */ });

    sync();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Refresh whenever the backend finishes a synchronization pass (background,
  // tray "Sync Now", or a provider recovering after a failure).
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let active = true;

    listen<SyncReport>('sync://complete', (event) => {
      setLastSync(event.payload);
      setSyncing(false);
      fetchDashboard();
    }).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [fetchDashboard]);

  return {
    data,
    timeRange,
    setTimeRange,
    providerFilter,
    setProviderFilter,
    loading,
    error,
    syncing,
    lastSync,
    sync,
    refresh: fetchDashboard,
  };
}
