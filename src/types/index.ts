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
  session_id?: string;
}

export interface ProviderStatus {
  id: string;
  name: string;
  connected: boolean;
  status: string;
  detail?: string;
  last_sync?: string;
}

export interface UsageSummary {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  request_count: number;
  estimated_api_value: number;
  actual_cost: number;
  free_value: number;
}

export interface ProviderUsage {
  provider: string;
  tokens: number;
  percentage: number;
  estimated_value: number;
}

export interface ModelUsage {
  model: string;
  tokens: number;
  percentage: number;
  estimated_value: number;
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

export type TimeRange = 'today' | 'week' | 'month' | 'all';

export function getDaysForRange(range: TimeRange): number | null {
  switch (range) {
    case 'today': return 1;
    case 'week': return 7;
    case 'month': return 30;
    case 'all': return null;
  }
}

export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(1)}M`;
  }
  if (tokens >= 1_000) {
    return `${(tokens / 1_000).toFixed(1)}K`;
  }
  return tokens.toLocaleString();
}

export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

export function formatTimestamp(ts: string): string {
  try {
    const date = new Date(ts);
    return date.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' });
  } catch {
    return '--:--:--';
  }
}
