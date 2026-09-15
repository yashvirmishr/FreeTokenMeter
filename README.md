# FreeTokenMeter

A lightweight desktop widget that monitors AI token usage across free coding/AI tools and estimates the equivalent API value of that usage.

## How Free Value Is Calculated

FreeTokenMeter separates **actual provider cost** from **reference API value**.

- **Actual Cost** — What the provider actually charged you for the usage. For free/promotional models, this is $0.
- **Reference API Value** — The equivalent cost of the same usage at a paid API rate. This is calculated using independently sourced pricing data.
- **Free Value** — `Reference API Value - Actual Cost`. This represents the economic equivalent value of the free usage.

For paid models where a real cost is reported, the reference value and actual cost may be the same (free_value = $0).

For free models like `mimo-v2.5-free` or `mimo/mimo-v2.5`, the actual cost is $0 while the reference value reflects what the same tokens would cost at paid MiMo-V2.5 API rates.

### Current Model Mappings

| Free Model | Provider | Reference Model | Reference Rate (per 1M tokens) |
|------------|----------|----------------|-------------------------------|
| `mimo-v2.5-free` | OpenCode | `mimo-v2.5` | Input: $0.14, Output: $0.28, Cached: $0.0028 |
| `mimo/mimo-v2.5` | Freebuff | `mimo-v2.5` | Input: $0.14, Output: $0.28, Cached: $0.0028 |

Reference pricing is sourced independently from the providers' promotional pricing and includes a verification date.

## Tech Stack

- **Tauri 2** — Desktop framework (Rust backend)
- **React 19** — UI framework
- **TypeScript** — Type safety
- **Vite 8** — Build tool
- **Tailwind CSS 4** — Styling
- **SQLite** — Local persistence (via rusqlite)

## Architecture

```
FreeTokenMeter/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── hooks/              # React hooks
│   └── types/              # TypeScript types
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # Tauri commands & state
│   │   ├── database.rs     # SQLite layer
│   │   ├── opencode_provider.rs  # OpenCode integration
│   │   ├── freebuff_provider.rs  # Freebuff integration
│   │   └── pricing.rs      # Pricing engine (actual vs reference)
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

It syncs session-level token usage data (input, output, cached, reasoning tokens) and calculates both actual cost and reference API value.

### Freebuff Integration

FreeTokenMeter reads from Freebuff Desktop's per-project SQLite databases at:

- **Windows**: `%USERPROFILE%\.config\freebuff-desktop\projects\<project-hash>\desktop-v2.db`

Each Freebuff project has its own database. FreeTokenMeter scans all project databases and aggregates usage data.

Data is extracted from the `messages` table where:
- `role = 'assistant'` (only assistant responses contain usage metrics)
- `metrics_json` is non-empty

The `metrics_json` field uses **camelCase** keys:

```json
{
  "usage": {
    "inputTokens": 11046756,
    "cachedInputTokens": 10823168,
    "outputTokens": 49162,
    "reasoningOutputTokens": 9789,
    "totalTokens": 11095918
  },
  "costUsd": 0
}
```

### Pricing Engine

The pricing engine (`src-tauri/src/pricing.rs`) maintains:

- **Actual pricing** — What each model costs via the provider (free models = $0)
- **Reference pricing** — What equivalent usage would cost at a paid API rate
- **Pricing profiles** — Per-model metadata including source, verification date, and version

Each pricing calculation produces:
- `actual_cost` — What you paid
- `reference_value` — What it would cost at reference rates
- `free_value` — `reference_value - actual_cost`
- `pricing_status` — How the pricing was determined
- `pricing_version` — Pricing engine version for historical consistency

### Database Schema

```sql
CREATE TABLE usage_records (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,          -- 'opencode' or 'freebuff'
    model TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost REAL NOT NULL DEFAULT 0.0,
    reference_value REAL,
    free_value REAL,
    pricing_status TEXT NOT NULL DEFAULT 'unknown',
    session_id TEXT,
    time_created INTEGER NOT NULL,
    pricing_version TEXT
);
```

### Pricing Engine Table

**OpenCode Models:**

| Model | Input | Output | Cached | Reference |
|-------|-------|--------|--------|-----------|
| mimo-v2.5-free | $0.00 | $0.00 | $0.00 | mimo-v2.5 ($0.14/$0.28/$0.0028) |
| nemotron-3.5-lightning-free | $0.00 | $0.00 | $0.00 | N/A |
| big-pickle | $0.00 | $0.00 | $0.00 | N/A |
| gpt-4o | $2.50 | $10.00 | $1.25 | self |
| gpt-4o-mini | $0.15 | $0.60 | $0.075 | self |
| claude-sonnet-4 | $3.00 | $15.00 | $0.30 | self |
| claude-haiku-3.5 | $0.80 | $4.00 | $0.08 | self |
| gemini-2.0-flash | $0.10 | $0.40 | $0.025 | self |
| deepseek-r1 | $0.55 | $2.19 | $0.14 | self |

**Freebuff Models:**

| Model | Input | Output | Cached | Reference |
|-------|-------|--------|--------|-----------|
| mimo/mimo-v2.5 | $0.00 | $0.00 | $0.00 | mimo-v2.5 ($0.14/$0.28/$0.0028) |
| z-ai/glm-5.3-flash | $0.00 | $0.00 | $0.00 | N/A |
| deepseek/deepseek-v4-flash | $0.00 | $0.00 | $0.00 | N/A |
| openai/gpt-5.6-luna | $0.00 | $0.00 | $0.00 | N/A |
| upstage/solar-pro4 | $0.00 | $0.00 | $0.00 | N/A |
| minimax/minimax-m3 | $0.00 | $0.00 | $0.00 | N/A |
| meta/muse-spark-1.2-contributor | $0.00 | $0.00 | $0.00 | N/A |
| meta/muse-spark-1.3-contributor | $0.00 | $0.00 | $0.00 | N/A |
| stealth/ox-alpha | $0.00 | $0.00 | $0.00 | N/A |
| google/gemini-3.8-flash | $0.00 | $0.00 | $0.00 | N/A |
| crof/kimi-k3-eco | $0.00 | $0.00 | $0.00 | N/A |
| z-ai/glm-5.2 | $0.00 | $0.00 | $0.00 | N/A |
| openai/gpt-5.6-luna-es | $0.00 | $0.00 | $0.00 | N/A |
| fable/fable-5 | $0.00 | $0.00 | $0.00 | N/A |

### UI Theme

Terminal-style dark theme:
- Background: `#0B0F0D`
- Primary text: `#D7FFD9`
- Terminal green: `#39FF88`
- Cyan accent: `#00E5FF`
- Muted text: `#617568`
- Warning: `#FFD166`
- Error: `#FF5C5C`
- Font: JetBrains Mono

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `r` | Sync all providers |
| `1` | Time range: Today |
| `2` | Time range: Week |
| `3` | Time range: Month |
| `4` | Time range: All Time |
| `q` | Provider filter: All |
| `w` | Provider filter: OpenCode |
| `e` | Provider filter: Freebuff |

## Adding a New Provider

1. Create a new file in `src-tauri/src/` (e.g., `my_provider.rs`)
2. Implement the provider interface:

```rust
pub struct MyProvider;

impl MyProvider {
    pub fn new() -> Self { Self }
    pub fn is_available(&self) -> bool { /* check if provider is installed */ }
    pub fn fetch_all_records(&self) -> Result<Vec<RawRecord>, String> { /* fetch data */ }
    pub fn get_status(&self) -> (bool, String, Option<String>) { /* return status */ }
}
```

3. Add a normalization function to convert raw records to `UsageRecord`
4. Register in `src-tauri/src/lib.rs`:
   - Add `mod my_provider;`
   - Add to `AppState`
   - Add sync command
   - Add status to `build_provider_status()`

## Adding a New Free Model Reference

To add a reference pricing mapping for a new free model:

1. Open `src-tauri/src/pricing.rs`
2. Add an explicit match arm in `lookup_profile()`:

```rust
"my-new-free-model" => Some(ModelPricingProfile {
    display_name: "My New Free Model",
    actual: PricingRates {
        input_per_million: 0.0,
        output_per_million: 0.0,
        cached_per_million: 0.0,
    },
    reference: Some(PricingRates {
        input_per_million: 0.10,  // Replace with actual reference price
        output_per_million: 0.40,
        cached_per_million: 0.025,
    }),
    reference_model: Some("my-new-model"),
    reference_source: "Provider API pricing documentation",
    reference_verified: "2025-07",
}),
```

**Important:** Do NOT assume that removing "-free" from a model ID gives you the paid equivalent. Always use explicit mappings.

## Known Limitations

- Pricing data may need updates for new models
- No system tray integration yet
- No always-on-top toggle in UI
- Window size not persisted across restarts

## Privacy

All usage data stays local. No data is sent to external servers.
