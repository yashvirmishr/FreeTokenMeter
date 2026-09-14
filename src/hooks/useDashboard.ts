import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { DashboardData, TimeRange } from '../types';
import { getDaysForRange } from '../types';

export function useDashboard() {
  const [data, setData] = useState<DashboardData | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>('today');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);

  const fetchDashboard = useCallback(async () => {
    try {
      const days = getDaysForRange(timeRange);
      const result = await invoke<DashboardData>('get_dashboard', { days });
      setData(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [timeRange]);

  const sync = useCallback(async () => {
    setSyncing(true);
    try {
      await invoke<string>('sync_opencode');
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
    loading,
    error,
    syncing,
    sync,
    refresh: fetchDashboard,
  };
}
