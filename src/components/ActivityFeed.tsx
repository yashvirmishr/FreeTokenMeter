import type { ActivityEntry } from '../types';
import { formatTokens, formatTimestamp } from '../types';

interface ActivityFeedProps {
  entries: ActivityEntry[];
}

export function ActivityFeed({ entries }: ActivityFeedProps) {
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
              <span className="provider">{entry.provider}</span>
              <span className="model">{entry.model}</span>
              <span className="tokens">+{formatTokens(entry.tokens)}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
