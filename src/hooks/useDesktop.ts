import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AppPreferences, FutureProviderView, WindowState } from '../types';

/**
 * Desktop shell state: the real native window state plus persisted preferences.
 *
 * The native window is the source of truth — every toggle applies to the window
 * first and this hook then stores what the window actually reports.
 */
export function useDesktop() {
  const [windowState, setWindowState] = useState<WindowState | null>(null);
  const [prefs, setPrefs] = useState<AppPreferences | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hiddenNotice, setHiddenNotice] = useState(false);
  const [futureProviders, setFutureProviders] = useState<FutureProviderView[]>([]);

  const refresh = useCallback(async () => {
    try {
      const [state, preferences] = await Promise.all([
        invoke<WindowState>('get_window_state'),
        invoke<AppPreferences>('get_preferences'),
      ]);
      setWindowState(state);
      setPrefs(preferences);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Providers that are documented as future work. Listed so a missing provider
  // reads as "not supported yet" rather than as a silent zero.
  useEffect(() => {
    invoke<FutureProviderView[]>('get_future_providers')
      .then(setFutureProviders)
      .catch(() => setFutureProviders([]));
  }, []);

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    const track = (fn: UnlistenFn) => {
      if (active) unlisteners.push(fn);
      else fn();
    };

    listen<WindowState>('prefs://changed', (event) => {
      setWindowState(event.payload);
      setPrefs((prev) => (prev ? { ...prev, always_on_top: event.payload.always_on_top } : prev));
    }).then(track);

    listen('window://hidden', () => setHiddenNotice(true)).then(track);

    return () => {
      active = false;
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  const setAlwaysOnTop = useCallback(async (enabled: boolean) => {
    try {
      const actual = await invoke<boolean>('set_always_on_top', { enabled });
      setWindowState((prev) => (prev ? { ...prev, always_on_top: actual } : prev));
      setPrefs((prev) => (prev ? { ...prev, always_on_top: actual } : prev));
      setError(null);
      return actual;
    } catch (err) {
      setError(String(err));
      return null;
    }
  }, []);

  const toggleAlwaysOnTop = useCallback(async () => {
    try {
      const actual = await invoke<boolean>('toggle_always_on_top');
      setWindowState((prev) => (prev ? { ...prev, always_on_top: actual } : prev));
      setPrefs((prev) => (prev ? { ...prev, always_on_top: actual } : prev));
      setError(null);
      return actual;
    } catch (err) {
      setError(String(err));
      return null;
    }
  }, []);

  /**
   * Enable/disable one provider. Persisted by the backend, so the choice
   * survives a restart and is honoured by the background scheduler.
   */
  const setProviderEnabled = useCallback(async (provider: string, enabled: boolean) => {
    try {
      const updated = await invoke<AppPreferences>('set_provider_enabled', { provider, enabled });
      setPrefs(updated);
      setError(null);
      return updated;
    } catch (err) {
      setError(String(err));
      return null;
    }
  }, []);

  /** First-import window (days) used by a provider with no cursor yet. */
  const setBackfillDays = useCallback(async (days: number) => {
    try {
      const updated = await invoke<AppPreferences>('set_backfill_days', { days });
      setPrefs(updated);
      setError(null);
      return updated;
    } catch (err) {
      setError(String(err));
      return null;
    }
  }, []);

  const hideToTray = useCallback(() => {
    invoke('hide_window').catch((err) => setError(String(err)));
  }, []);

  const showWindow = useCallback(() => {
    invoke('show_window').catch((err) => setError(String(err)));
  }, []);

  const quit = useCallback(() => {
    invoke('quit_app').catch((err) => setError(String(err)));
  }, []);

  return {
    windowState,
    prefs,
    futureProviders,
    error,
    hiddenNotice,
    dismissHiddenNotice: () => setHiddenNotice(false),
    setAlwaysOnTop,
    toggleAlwaysOnTop,
    setProviderEnabled,
    setBackfillDays,
    hideToTray,
    showWindow,
    quit,
    refresh,
  };
}
