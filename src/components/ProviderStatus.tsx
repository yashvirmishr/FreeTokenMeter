import type { ProviderStatus as ProviderStatusType } from '../types';

interface ProviderStatusProps {
  providers: ProviderStatusType[];
}

export function ProviderStatus({ providers }: ProviderStatusProps) {
  return (
    <div className="terminal-box">
      <div className="terminal-header">PROVIDERS</div>
      <div className="mt-2 space-y-3">
        {providers.map((p) => (
          <div key={p.id}>
            <div className="flex items-center gap-2 mb-1">
              <span className={`w-2 h-2 rounded-full ${p.connected ? 'bg-green' : 'bg-error'}`} />
              <span className="text-text font-medium text-sm">{p.name}</span>
              <span className={`text-xs ${p.connected ? 'text-green' : 'text-error'}`}>
                STATUS: {p.status}
              </span>
            </div>
            {p.detail && (
              <div className="text-muted text-xs ml-4">{p.detail}</div>
            )}
            {p.last_sync && (
              <div className="text-muted text-xs ml-4">
                LAST SYNC: {p.last_sync}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
