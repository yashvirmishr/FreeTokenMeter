import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { DashboardData, TimeRange } from '../types';
import { getDaysForRange } from '../types';

export type ProviderFilter = 'all' | 'opencode' | 'freebuff';

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>('today');
  const [providerFilter, setProviderFilter] = useState<ProviderFilter>('all');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);

  const fetchDashboard = useCallback(async () => {
    try {
      const days = getDaysForRange(timeRange);
      const provider = providerFilter === 'all' ? null : providerFilter;
      const result = await invoke<DashboardData>('get_dashboard', { days, provider });
      setData(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [timeRange, providerFilter]);

  const sync = useCallback(async () => {
    setSyncing(true);
    try {
      await invoke<string>('sync_all');
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

  // Auto-sync every 30 seconds
  useEffect(() => {
    const interval = setInterval(() => {
      sync();
    }, 30000);
    return () => clearInterval(interval);
  }, [sync]);

  return {
    data,
    timeRange,
    setTimeRange,
    providerFilter,
    setProviderFilter,
    loading,
    error,
    syncing,
    sync,
    refresh: fetchDashboard,
  };
}
