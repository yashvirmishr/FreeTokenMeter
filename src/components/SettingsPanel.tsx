import type { AppPreferences, FutureProviderView, ProviderStatus, WindowState } from '../types';
import { providerGlyph, providerGlyphClass } from '../types';

interface SettingsPanelProps {
  windowState: WindowState | null;
  prefs: AppPreferences | null;
  providers: ProviderStatus[];
  /** Providers with no supported local source, documented rather than faked. */
  futureProviders: FutureProviderView[];
  pricingVersion: string | null;
  registryModels: number;
  syncIntervalSecs: number;
  onToggleAlwaysOnTop: () => void;
  onToggleProvider: (id: string, enabled: boolean) => void;
  onSetBackfillDays: (days: number) => void;
  onHide: () => void;
  onQuit: () => void;
  onClose: () => void;
}

/** First-import windows offered in the UI. `0` means all available history. */
const BACKFILL_CHOICES: { days: number; label: string }[] = [
  { days: 7, label: '7 DAYS' },
  { days: 30, label: '30 DAYS' },
  { days: 0, label: 'ALL AVAILABLE' },
];

/**
 * `$ settings` — terminal-style configuration. Every value shown here is the
 * real state reported by the native window, the preferences file or the
 * provider registry.
 */
export function SettingsPanel({
  windowState,
  prefs,
  providers,
  futureProviders,
  pricingVersion,
  registryModels,
  syncIntervalSecs,
  onToggleAlwaysOnTop,
  onToggleProvider,
  onSetBackfillDays,
  onHide,
  onQuit,
  onClose,
}: SettingsPanelProps) {
  const alwaysOnTop = windowState?.always_on_top ?? prefs?.always_on_top ?? false;
  const backfillDays = prefs?.backfill_days ?? 30;
  const geometry = windowState
    ? `${windowState.width}x${windowState.height}${
        windowState.x != null && windowState.y != null ? ` @ (${windowState.x}, ${windowState.y})` : ''
      }`
    : '—';

  return (
    <div className="terminal-box fade-in">
      <div className="flex items-center justify-between">
        <span className="terminal-header border-0 p-0">$ settings</span>
        <button
          onClick={onClose}
          className="text-xs text-muted hover:text-green transition-colors"
          title="Close [s]"
        >
          [x]
        </button>
      </div>

      <div className="mt-2 space-y-1 text-xs">
        <SettingRow
          hint="1"
          label="Always on Top"
          value={alwaysOnTop ? 'ON' : 'OFF'}
          valueClass={alwaysOnTop ? 'text-green' : 'text-muted'}
          onClick={onToggleAlwaysOnTop}
        />
        <SettingRow
          hint=" "
          label="Background Sync"
          value={`every ${syncIntervalSecs}s`}
          valueClass="text-green"
        />
        <SettingRow
          hint=" "
          label="Window Geometry"
          value={geometry}
          valueClass="text-text"
        />
        <SettingRow
          hint=" "
          label="Preferences"
          value="preferences.json (local)"
          valueClass="text-text"
        />
        <SettingRow
          hint=" "
          label="Pricing Registry"
          value={pricingVersion ?? '—'}
          valueClass="text-cyan"
        />
        <SettingRow
          hint=" "
          label="Pricing Models"
          value={`${registryModels}`}
          valueClass="text-cyan"
        />
      </div>

      <div className="divider" />

      <div className="terminal-header border-0 p-0 text-xs">$ settings providers</div>
      <div className="mt-2 space-y-1 text-xs">
        {providers.map((provider) => (
          <button
            key={provider.id}
            onClick={() => onToggleProvider(provider.id, !provider.enabled)}
            className="flex items-center gap-2 px-1 py-0.5 w-full hover:bg-bg-lighter transition-colors text-left"
            title={
              provider.connected
                ? `Synchronize ${provider.name}`
                : `${provider.name} cannot synchronize: ${provider.status}`
            }
          >
            <span className="text-muted">[{provider.enabled ? '✓' : ' '}]</span>
            <span className="text-text flex-1">{provider.name}</span>
            <span className={providerGlyphClass(provider.state)} aria-hidden>
              {providerGlyph(provider.state)}
            </span>
          </button>
        ))}
      </div>

      {futureProviders.length > 0 && (
        <>
          <div className="divider" />
          <div className="terminal-header border-0 p-0 text-xs">$ providers future</div>
          <div className="mt-2 space-y-2 text-xs">
            {futureProviders.map((provider) => (
              <div key={provider.id}>
                <div className="flex items-center gap-2">
                  <span className="text-warning" aria-hidden>
                    ?
                  </span>
                  <span className="text-text">{provider.name}</span>
                  <span className="text-warning">UNSUPPORTED</span>
                </div>
                <div className="text-muted ml-5">{provider.reason}</div>
              </div>
            ))}
          </div>
        </>
      )}

      <div className="divider" />

      <div className="terminal-header border-0 p-0 text-xs">$ settings backfill</div>
      <div className="mt-2 flex gap-2 flex-wrap text-xs">
        {BACKFILL_CHOICES.map((choice) => (
          <button
            key={choice.days}
            onClick={() => onSetBackfillDays(choice.days)}
            className={`tab ${backfillDays === choice.days ? 'active' : ''}`}
            title="History imported the first time a provider syncs"
          >
            {choice.label}
          </button>
        ))}
      </div>
      <div className="mt-1 text-xs text-muted">
        Applies to providers that have not synced yet. Existing history is never re-imported.
      </div>

      <div className="divider" />

      <div className="flex gap-2 text-xs">
        <button onClick={onHide} className="tab" title="Hide to tray [h]">
          [h] HIDE TO TRAY
        </button>
        <button onClick={onQuit} className="tab" title="Quit FreeTokenMeter [x]">
          [x] QUIT
        </button>
      </div>

      <div className="mt-2 pt-2 border-t border-border text-xs text-muted">
        Closing the window hides it to the system tray. Background sync keeps
        collecting usage until you quit.
      </div>
    </div>
  );
}

function SettingRow({
  hint,
  label,
  value,
  valueClass,
  onClick,
}: {
  hint: string;
  label: string;
  value: string;
  valueClass: string;
  onClick?: () => void;
}) {
  const content = (
    <>
      <span className="text-muted">[{hint}]</span>
      <span className="text-text flex-1 text-left">{label}</span>
      <span className={valueClass}>{value}</span>
    </>
  );

  if (!onClick) {
    return <div className="flex items-center gap-2 px-1 py-0.5">{content}</div>;
  }

  return (
    <button
      onClick={onClick}
      className="flex items-center gap-2 px-1 py-0.5 w-full hover:bg-bg-lighter transition-colors"
    >
      {content}
    </button>
  );
}
