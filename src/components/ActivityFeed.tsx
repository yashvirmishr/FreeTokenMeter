import type { ActivityEntry, ProviderStatus } from '../types';
import { formatTokens, formatTimestamp } from '../types';

interface ActivityFeedProps {
  entries: ActivityEntry[];
  status: ProviderStatus[];
}

/** Per-provider accent so a multi-provider feed stays scannable. */
const PROVIDER_COLOURS: Record<string, string> = {
  opencode: 'text-green',
  freebuff: 'text-cyan',
  'claude-code': 'text-warning',
  codex: 'text-green',
  'gemini-cli': 'text-cyan',
  'github-copilot': 'text-warning',
};

export function ActivityFeed({ entries, status }: ActivityFeedProps) {
  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();

  return (
    <div className="terminal-box">
      <div className="terminal-header">ACTIVITY</div>
      <div className="mt-2 max-h-32 overflow-y-auto">
        {entries.length === 0 ? (
          <div className="text-muted text-xs py-2">No recent activity</div>
        ) : (
          entries.slice(0, 10).map((entry, i) => (
            <div key={i} className="activity-row fade-in">
              <span className="time">{formatTimestamp(entry.timestamp)}</span>
              <span className={`provider ${PROVIDER_COLOURS[entry.provider] ?? 'text-text'}`}>
                [{label(entry.provider)}]
              </span>
              <span className="model">{entry.model}</span>
              <span className="tokens">+{formatTokens(entry.tokens)}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
