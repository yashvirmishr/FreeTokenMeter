export interface UsageRecord {
  id: string;
  provider: string;
  model: string;
  timestamp: string;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
  estimated_cost: number;
  reference_value: number | null;
  free_value: number | null;
  pricing_status: string;
  session_id: string | null;
  pricing_version: string | null;
}

export interface ProviderStatus {
  id: string;
  name: string;
  connected: boolean;
  status: string;
  detail: string | null;
  last_sync: string | null;
}

export interface UsageSummary {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  request_count: number;
  actual_cost: number;
  reference_value: number;
  free_value: number;
}

export interface ProviderUsage {
  provider: string;
  tokens: number;
  percentage: number;
  actual_cost: number;
  reference_value: number;
}

export interface ModelUsage {
  model: string;
  tokens: number;
  percentage: number;
  actual_cost: number;
  reference_value: number | null;
  pricing_status: string;
}

export interface ActivityEntry {
  timestamp: string;
  provider: string;
  model: string;
  tokens: number;
}

export interface DashboardData {
  summary: UsageSummary;
  provider_usage: ProviderUsage[];
  model_usage: ModelUsage[];
  activity: ActivityEntry[];
  providers: ProviderStatus[];
}

// ── Formatting helpers ──

export type TimeRange = 'today' | 'week' | 'month' | 'all';

export function getDaysForRange(range: TimeRange): number | null {
  switch (range) {
    case 'today': return 1;
    case 'week': return 7;
    case 'month': return 30;
    case 'all': return null;
  }
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

export function formatCurrency(n: number): string {
  if (n === 0) return '$0.00';
  if (n < 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

export function formatTimestamp(ts: string): string {
  const date = new Date(ts);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);

  if (diffMins < 1) return 'now';
  if (diffMins < 60) return `${diffMins}m`;
  const diffHrs = Math.floor(diffMins / 60);
  if (diffHrs < 24) return `${diffHrs}h`;
  const diffDays = Math.floor(diffHrs / 24);
  return `${diffDays}d`;
}
