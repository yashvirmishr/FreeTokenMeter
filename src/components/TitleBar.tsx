import { RegistryStatus } from './RegistryStatus';

interface TitleBarProps {
  syncing: boolean;
  alwaysOnTop: boolean;
  settingsOpen: boolean;
  pricingOpen: boolean;
  onSync: () => void;
  onToggleSettings: () => void;
  onTogglePricing: () => void;
  onHide: () => void;
}

export function TitleBar({
  syncing,
  alwaysOnTop,
  settingsOpen,
  pricingOpen,
  onSync,
  onToggleSettings,
  onTogglePricing,
  onHide,
}: TitleBarProps) {
  return (
    <div className="flex items-center justify-between px-3 sm:px-4 py-2 border-b border-border bg-bg-light select-none">
      <div className="flex items-center gap-2 sm:gap-3 min-w-0">
        <span className="text-green text-xs font-bold tracking-wider">FreeTokenMeter</span>
        <span className="text-muted text-xs hidden sm:inline">v0.1.0</span>
        {alwaysOnTop && (
          <span className="text-xs text-cyan" title="Always on top is enabled">
            PINNED
          </span>
        )}
      </div>

      <div className="flex items-center gap-2 sm:gap-3">
        <RegistryStatus />
        <button
          onClick={onSync}
          disabled={syncing}
          className="text-xs text-muted hover:text-green transition-colors disabled:opacity-50"
          title="Sync all providers [r]"
        >
          {syncing ? '⟳ SYNCING...' : '↻ SYNC'}
        </button>
        <button
          onClick={onTogglePricing}
          className={`text-xs transition-colors ${pricingOpen ? 'text-green' : 'text-muted hover:text-green'}`}
          title="Pricing registry [p]"
        >
          [$]
        </button>
        <button
          onClick={onToggleSettings}
          className={`text-xs transition-colors ${settingsOpen ? 'text-green' : 'text-muted hover:text-green'}`}
          title="Settings [s]"
        >
          [⚙]
        </button>
        <button
          onClick={onHide}
          className="text-xs text-muted hover:text-green transition-colors"
          title="Hide to system tray [h]"
        >
          [⤓]
        </button>
        <span className="flex items-center gap-1.5 text-xs" title="Background sync runs every 30s">
          <span className={`w-2 h-2 rounded-full ${syncing ? 'bg-warning pulse' : 'bg-green'}`} />
          <span className="text-muted">{syncing ? 'SYNCING' : 'RUNNING'}</span>
        </span>
      </div>
    </div>
  );
}
