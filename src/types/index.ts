// Normalized usage model + dashboard shapes.
//
// Mirrors `src-tauri/src/usage.rs`. Two rules are visible throughout:
//   1. `null` means "not reported" — it is never the same as `0`.
//   2. Billing units (AI credits, plan units) are separate from USD.

// ── Providers ──

/** Detection states. Only `connected` means a provider contributes records. */
export type ProviderState =
  | 'connected'
  | 'not_installed'
  | 'usage_unavailable'
  | 'disabled'
  | 'unsupported'
  | 'error';

export interface ProviderStatus {
  /** Canonical id, e.g. `claude-code`. */
  id: string;
  name: string;
  /** Short uppercase label for narrow columns, e.g. `CLAUDE`. */
  short_label: string;
  state: ProviderState;
  connected: boolean;
  /** Terminal-style label: CONNECTED, USAGE UNAVAILABLE, … */
  status: string;
  detail: string | null;
  enabled: boolean;
  /** Structured source kind, e.g. `codex_session_file`. */
  source_type: string;
  /** `api` | `subscription` | `promotional` | `unknown` */
  payment_mode: string;
  /** `usd` | `ai_credits` | `subscription_units` | `unknown` */
  billing_unit: string;
  /** Human name of the metered unit, e.g. `AI credits`; null for plain USD. */
  billing_metric_name: string | null;
  /** Version of the tool that produced this provider's records, when known. */
  source_version: string | null;
  last_sync_age_secs: number | null;
  last_sync: string | null;
  records: number;
  tokens: number;
}

export interface ProviderDescriptor {
  id: string;
  name: string;
  short_label: string;
  source_type: string;
  payment_mode: string;
  billing_unit: string;
  state: ProviderState;
  status: string;
  detail: string | null;
  default_backfill_days: number;
}

export interface FutureProviderView {
  id: string;
  name: string;
  reason: string;
}

// ── Usage ──

export interface UsageRecord {
  id: string;
  provider: string;
  model: string;
  timestamp: string;
  input_tokens: number | null;
  output_tokens: number | null;
  reasoning_tokens: number | null;
  cached_tokens: number | null;
  cache_write_tokens: number | null;
  total_tokens: number;
  actual_cost_usd: number | null;
  reference_value: number | null;
  free_value: number | null;
  billing_unit: string;
  billing_units: number | null;
  /** Name of the metered unit as the provider reports it, e.g. `AI credits`. */
  billing_metric_name: string | null;
  payment_mode: string;
  pricing_status: string;
  pricing_source: string | null;
  pricing_version: string | null;
  reference_model: string | null;
  session_id: string | null;
  event_id: string | null;
  source_type: string;
  source_id: string | null;
  /** Version of the tool that wrote the source data, when the source has it. */
  source_version: string | null;
}

export interface UsageSummary {
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  request_count: number;
  /** Only records with a known monetary cost contribute. */
  actual_cost: number;
  /** Records that reported no monetary cost at all (subscription usage). */
  unbilled_records: number;
  reference_value: number;
  free_value: number;
  /** Billed units across providers whose unit is not USD. */
  billing_units: number;
  unpriced_records: number;
  unpriced_tokens: number;
}

export interface ProviderUsage {
  provider: string;
  tokens: number;
  percentage: number;
  actual_cost: number | null;
  reference_value: number | null;
  billing_units: number | null;
  billing_unit: string;
  /** Name of the metered unit, e.g. `AI credits`; null for plain USD. */
  billing_metric_name: string | null;
  records: number;
}

export interface ModelUsage {
  provider: string;
  model: string;
  tokens: number;
  percentage: number;
  actual_cost: number | null;
  reference_value: number | null;
  billing_units: number | null;
  billing_unit: string;
  /** Name of the metered unit, e.g. `AI credits`; null for plain USD. */
  billing_metric_name: string | null;
  pricing_status: string;
  pricing_source: string | null;
}

export interface ActivityEntry {
  timestamp: string;
  provider: string;
  model: string;
  tokens: number;
  billing_units: number | null;
  billing_unit: string;
}

export interface DashboardData {
  summary: UsageSummary;
  provider_usage: ProviderUsage[];
  model_usage: ModelUsage[];
  activity: ActivityEntry[];
  providers: ProviderStatus[];
}

// ── Desktop shell (window / preferences / sync) ──

export interface ProviderSyncResult {
  provider: string;
  name: string;
  ok: boolean;
  /** Not synced at all: disabled by the user or no readable source. */
  skipped: boolean;
  message: string;
  records_synced: number;
  new_tokens: number;
  events_scanned: number;
  events_skipped: number;
  duration_ms: number;
}

export interface SyncReport {
  providers: ProviderSyncResult[];
  started_at: number;
  finished_at: number;
  new_tokens: number;
}

export interface AppPreferences {
  always_on_top: boolean;
  window_width: number;
  window_height: number;
  window_x: number | null;
  window_y: number | null;
  /** Providers switched off. Empty means nothing is disabled. */
  disabled_providers: string[];
  /** First-import window in days; 0 means all available history. */
  backfill_days: number;
}

export interface WindowState {
  always_on_top: boolean;
  visible: boolean;
  width: number;
  height: number;
  x: number | null;
  y: number | null;
}

// ── Pricing registry ──

export interface PricingRates {
  input_per_million: number;
  output_per_million: number;
  cached_per_million: number;
}

export interface PricingProfileView {
  provider: string;
  model_id: string;
  display_name: string;
  actual: PricingRates;
  reference: PricingRates | null;
  reference_model: string | null;
  source: string;
  version: string;
  verified_at: string;
  is_free: boolean;
}

export interface UnknownModelView {
  provider: string;
  model: string;
  tokens: number;
  records: number;
}

export interface PricingRegistryView {
  version: string;
  registry_version: string | null;
  registry_models: number;
  known: PricingProfileView[];
  unknown: UnknownModelView[];
}

export interface RegistryStatusView {
  model_count: number;
  last_updated: string | null;
  cache_age: string | null;
  needs_refresh: boolean;
}

/** Records priced by an unknown model — reported as unpriced, never as $0. */
export const PRICING_STATUS_UNKNOWN = 'unknown';

export function isUnpriced(status: string): boolean {
  return status === PRICING_STATUS_UNKNOWN;
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

/** `null` renders as N/A: an unestablished cost is not $0.00. */
export function formatCurrency(n: number | null | undefined): string {
  if (n == null) return 'N/A';
  if (n === 0) return '$0.00';
  if (n < 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

/** Billed units for non-USD providers, e.g. `14 cr`. */
export function formatBilling(units: number | null | undefined, unit: string): string {
  if (units == null || units === 0) return '—';
  const suffix = unit === 'ai_credits' ? ' cr' : unit === 'subscription_units' ? ' units' : '';
  return `${trimNumber(units)}${suffix}`;
}

function trimNumber(n: number): string {
  return Number.isInteger(n) ? n.toString() : n.toFixed(2);
}

export function formatRate(rates: PricingRates): string {
  return `$${rates.input_per_million}/$${rates.output_per_million}/$${rates.cached_per_million}`;
}

/** "in / out / cached" per-million rates for terminal-style rows. */
export function formatRateDetail(rates: PricingRates): string {
  return `in ${rates.input_per_million} / out ${rates.output_per_million} / cached ${rates.cached_per_million}`;
}

export function formatThousands(n: number): string {
  return n.toLocaleString('en-US');
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

/** Payment-mode label shown next to a provider. */
export function paymentModeLabel(mode: string): string {
  switch (mode) {
    case 'api': return 'API';
    case 'subscription': return 'SUBSCRIPTION';
    case 'promotional': return 'PROMOTIONAL';
    default: return 'UNKNOWN';
  }
}

/** Marker glyph per detection state. */
export function providerGlyph(state: ProviderState): string {
  switch (state) {
    case 'connected': return '●';
    case 'disabled': return '−';
    case 'not_installed': return '○';
    case 'usage_unavailable': return '!';
    case 'unsupported': return '?';
    case 'error': return '!';
  }
}

export function providerGlyphClass(state: ProviderState): string {
  switch (state) {
    case 'connected': return 'text-green';
    case 'disabled': return 'text-muted';
    case 'error': return 'text-error';
    default: return 'text-warning';
  }
}
