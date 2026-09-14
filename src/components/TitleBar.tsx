interface TitleBarProps {
  syncing: boolean;
  onSync: () => void;
}

export function TitleBar({ syncing, onSync }: TitleBarProps) {
  return (
    <div className="flex items-center justify-between px-4 py-2 border-b border-border bg-bg-light select-none">
      <div className="flex items-center gap-3">
        <span className="text-green text-xs font-bold tracking-wider">
          FreeTokenMeter
        </span>
        <span className="text-muted text-xs">
          v0.1.0
        </span>
      </div>
      <div className="flex items-center gap-3">
        <button
          onClick={onSync}
          disabled={syncing}
          className="text-xs text-muted hover:text-green transition-colors disabled:opacity-50"
        >
          {syncing ? '⟳ SYNCING...' : '↻ SYNC'}
        </button>
        <span className="flex items-center gap-1.5 text-xs">
          <span className={`w-2 h-2 rounded-full ${syncing ? 'bg-warning pulse' : 'bg-green'}`} />
          {syncing ? 'SYNCING' : 'RUNNING'}
        </span>
      </div>
    </div>
  );
}
