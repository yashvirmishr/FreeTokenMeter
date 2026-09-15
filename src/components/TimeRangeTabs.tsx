import { useEffect } from 'react';
import type { TimeRange } from '../types';

interface TimeRangeTabsProps {
  active: TimeRange;
  onChange: (range: TimeRange) => void;
  /** Panels own the number keys while they are open. */
  disabled?: boolean;
}

const ranges: { key: TimeRange; label: string; keyHint: string }[] = [
  { key: 'today', label: 'TODAY', keyHint: '1' },
  { key: 'week', label: 'WEEK', keyHint: '2' },
  { key: 'month', label: 'MONTH', keyHint: '3' },
  { key: 'all', label: 'ALL TIME', keyHint: '4' },
];

export function TimeRangeTabs({ active, onChange, disabled = false }: TimeRangeTabsProps) {
  useEffect(() => {
    if (disabled) return;
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      switch (e.key) {
        case '1': onChange('today'); break;
        case '2': onChange('week'); break;
        case '3': onChange('month'); break;
        case '4': onChange('all'); break;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onChange, disabled]);

  return (
    <div className="flex gap-2 justify-center py-1">
      {ranges.map((r) => (
        <button
          key={r.key}
          onClick={() => onChange(r.key)}
          className={`tab ${active === r.key ? 'active' : ''}`}
        >
          [{r.keyHint}] {r.label}
        </button>
      ))}
    </div>
  );
}
