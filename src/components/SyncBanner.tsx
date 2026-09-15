import type { ProviderSyncResult, SyncReport } from '../types';

interface SyncBannerProps {
  report: SyncReport | null;
  hiddenNotice: boolean;
  onDismissHidden: () => void;
}

/**
 * Terminal-style status line for tray-triggered and background syncs:
 *
 *   SYNC COMPLETE
 *   OpenCode: +12,482 tokens
 *   Codex: +18,931 tokens
 *   GitHub Copilot: no readable source
 */
export function SyncBanner({ report, hiddenNotice, onDismissHidden }: SyncBannerProps) {
  if (hiddenNotice) {
    return (
      <div className="terminal-box text-xs flex items-start gap-2">
        <span className="text-warning">TRAY</span>
        <span className="text-muted flex-1">
          Window hidden — FreeTokenMeter is still running in the system tray and
          background synchronization continues.
        </span>
        <button
          onClick={onDismissHidden}
          className="text-muted hover:text-green transition-colors"
          title="Dismiss"
        >
          [x]
        </button>
      </div>
    );
  }

  if (!report) return null;

  const failed = report.providers.some((p) => !p.ok && !p.skipped);
  const attempted = report.providers.filter((p) => !p.skipped);

  return (
    <div className="terminal-box text-xs fade-in">
      <div className="flex items-center justify-between mb-1">
        <span className={failed ? 'text-warning' : 'text-green'}>
          {failed ? 'SYNC COMPLETE (WITH ERRORS)' : 'SYNC COMPLETE'}
        </span>
        <span className="text-muted">
          {attempted.length} provider(s) · {formatDuration(report)}
        </span>
      </div>
      <div className="text-muted flex flex-col gap-0.5">
        {report.providers.map((result) => (
          <span key={result.provider} className={lineClass(result)}>
            {formatLine(result)}
          </span>
        ))}
      </div>
    </div>
  );
}

function lineClass(result: ProviderSyncResult): string {
  if (result.skipped) return 'text-muted';
  return result.ok ? 'text-text' : 'text-error';
}

function formatLine(result: ProviderSyncResult): string {
  if (result.skipped) return `${result.name}: ${result.message}`;
  if (!result.ok) return `${result.name}: FAILED - ${result.message}`;
  return `${result.name}: +${result.new_tokens.toLocaleString('en-US')} tokens`;
}

function formatDuration(report: SyncReport): string {
  const ms = Math.max(0, report.finished_at - report.started_at);
  if (ms === 0) return '';
  return `${ms}ms`;
}
