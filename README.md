# FreeTokenMeter

A lightweight desktop widget that monitors AI token usage across free coding/AI tools and estimates the equivalent API value of that usage.

## Tech Stack

- **Tauri 2** - Desktop framework (Rust backend)
- **React 19** - UI framework
- **TypeScript** - Type safety
- **Vite 8** - Build tool
- **Tailwind CSS 4** - Styling
- **SQLite** - Local persistence (via rusqlite)

## Architecture

```
FreeTokenMeter/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── hooks/              # React hooks
│   ├── types/              # TypeScript types
│   └── lib/                # Utilities
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # Tauri commands & state
│   │   ├── database.rs     # SQLite layer
│   │   ├── opencode_provider.rs  # OpenCode integration
│   │   └── freebuff_provider.rs  # Freebuff stub
│   ├── migrations/         # DB migrations
│   └── tauri.conf.json     # Tauri config
└── public/                 # Static assets
```

## Setup

### Prerequisites

- Node.js 18+
- Rust (stable, MSVC toolchain on Windows)
- Tauri CLI: `npm install -D @tauri-apps/cli`

### Install

```bash
npm install
```

### Development

```bash
npm run tauri dev
```

### Build

```bash
npm run tauri build
```

## How It Works

### OpenCode Integration

FreeTokenMeter reads directly from OpenCode's SQLite database at:

- **Windows**: `%USERPROFILE%\.local\share\opencode\opencode.db`
- **macOS**: `~/.local/share/opencode/opencode.db`
- **Linux**: `~/.local/share/opencode/opencode.db`

It syncs session-level token usage data (input, output, cached tokens) and calculates estimated API values using model-specific pricing.

### Provider Architecture

Providers implement the `UsageProvider` interface:

```rust
trait UsageProvider {
    fn is_available(&self) -> bool;
    fn sync_usage(&self, last_sync: Option<i64>) -> Result<Vec<UsageRecord>, String>;
    fn get_status(&self) -> (bool, String, Option<String>);
}
```

Currently implemented:
- **OpenCode** - Reads from local SQLite database
- **Freebuff** - Stub only (not yet implemented)

### Database Schema

```sql
CREATE TABLE usage_records (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost REAL NOT NULL DEFAULT 0.0,
    session_id TEXT,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);
```

### Pricing Engine

Model-specific pricing (per million tokens, USD):

| Model | Input | Output | Cached |
|-------|-------|--------|--------|
| mimo-v2.5-free | $0.00 | $0.00 | $0.00 |
| nemotron-3-ultra-free | $0.00 | $0.00 | $0.00 |
| gpt-4o | $2.50 | $10.00 | $1.25 |
| gpt-4o-mini | $0.15 | $0.60 | $0.075 |
| claude-sonnet-4 | $3.00 | $15.00 | $0.30 |
| claude-haiku-3.5 | $0.80 | $4.00 | $0.08 |
| gemini-2.0-flash | $0.10 | $0.40 | $0.025 |
| deepseek-r1 | $0.55 | $2.19 | $0.14 |

### UI Theme

Terminal-style dark theme:
- Background: `#0B0F0D`
- Primary text: `#D7FFD9`
- Terminal green: `#39FF88`
- Muted text: `#617568`
- Warning: `#FFD166`
- Error: `#FF5C5C`
- Font: JetBrains Mono

## Adding a New Provider

1. Create a new file in `src-tauri/src/` (e.g., `my_provider.rs`)
2. Implement the provider interface:

```rust
pub struct MyProvider;

impl MyProvider {
    pub fn new() -> Self { Self }
    pub fn is_available(&self) -> bool { /* check if provider is installed */ }
    pub fn sync_usage(&self, last_sync: Option<i64>) -> Result<Vec<UsageRecord>, String> { /* fetch data */ }
    pub fn get_status(&self) -> (bool, String, Option<String>) { /* return status */ }
}
```

3. Register in `src-tauri/src/lib.rs`:
   - Add `mod my_provider;`
   - Add to `AppState`
   - Add status to `get_dashboard` and `get_provider_status`

## Known Limitations

- Freebuff integration not yet implemented
- Pricing data may need updates for new models
- No system tray integration yet
- No always-on-top toggle in UI
- Window size not persisted across restarts

## Privacy

All usage data stays local. No data is sent to external servers.
