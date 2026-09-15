import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { PricingRegistryView } from '../types';

/**
 * Loads the centralized pricing registry plus any model seen in local history
 * that is not priced yet. Reloaded whenever the panel is opened so newly
 * observed unknown models show up without restarting.
 */
export function usePricingRegistry(active: boolean) {
  const [registry, setRegistry] = useState<PricingRegistryView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const view = await invoke<PricingRegistryView>('get_pricing_registry');
      setRegistry(view);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active) load();
  }, [active, load]);

  return { registry, loading, error, reload: load };
}
