mod database;
mod opencode_provider;
mod pricing;
mod freebuff_provider;

use std::sync::Mutex;
use tauri::State;
use chrono::Utc;
use log::{info, error};

use database::{Database, DashboardData, ProviderStatus};

struct AppState {
    db: Mutex<Database>,
    opencode: opencode_provider::OpenCodeProvider,
    freebuff: freebuff_provider::FreebuffProvider,
}

fn get_data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
        .join("freetokenmeter")
}

/// Format a timestamp (millis since epoch) into a human-readable "X ago" string.
fn format_time_ago(millis: i64) -> String {
    let diff = Utc::now().timestamp_millis() - millis;
    if diff < 0 {
        return "just now".to_string();
    }
    if diff < 60_000 {
        format!("{}s ago", diff / 1000)
    } else if diff < 3_600_000 {
        format!("{}m ago", diff / 60_000)
    } else if diff < 86_400_000 {
        format!("{}h ago", diff / 3_600_000)
    } else {
        format!("{}d ago", diff / 86_400_000)
    }
}

/// Build the provider status list. Shared by all commands that need it.
fn build_provider_status(state: &AppState) -> Vec<ProviderStatus> {
    let db = state.db.lock().unwrap();
    let mut providers = Vec::new();

    // OpenCode
    let (oc_connected, oc_status, oc_detail) = state.opencode.get_status();
    let oc_last_sync = db.get_last_sync_time("opencode").ok().flatten()
        .map(format_time_ago);

    providers.push(ProviderStatus {
        id: "opencode".to_string(),
        name: "OpenCode".to_string(),
        connected: oc_connected,
        status: oc_status,
        detail: oc_detail,
        last_sync: oc_last_sync,
    });

    // Freebuff
    let (fb_connected, fb_status, fb_detail) = state.freebuff.get_status();
    let fb_last_sync = db.get_last_sync_time("freebuff").ok().flatten()
        .map(format_time_ago);

    providers.push(ProviderStatus {
        id: "freebuff".to_string(),
        name: "Freebuff".to_string(),
        connected: fb_connected,
        status: fb_status,
        detail: fb_detail,
        last_sync: fb_last_sync,
    });

    providers
}

#[tauri::command]
fn sync_opencode(state: State<'_, AppState>) -> Result<String, String> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    let last_sync = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_last_sync_time("opencode").map_err(|e| e.to_string())?
    };

    // Fetch sessions from OpenCode (outside the DB lock)
    let sessions = state.opencode.fetch_sessions(last_sync)?;

    // Normalize to UsageRecords
    let records: Vec<database::UsageRecord> = sessions.iter()
        .map(opencode_provider::OpenCodeProvider::normalize_session)
        .collect();

    // Persist to our database
    let (synced, total_records) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let count = db.upsert_usage_records(&records).map_err(|e| e.to_string())?;
        let total = db.count_records().map_err(|e| e.to_string())?;
        (count, total)
    };

    // Update sync timestamp
    if !records.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let now = Utc::now().timestamp_millis();
        db.update_sync_state("opencode", now).map_err(|e| e.to_string())?;
    }

    info!("OpenCode sync complete: {} records upserted, {} total in database", synced, total_records);
    Ok(format!("Synced {} records ({} total)", synced, total_records))
}

#[tauri::command]
fn sync_freebuff(state: State<'_, AppState>) -> Result<String, String> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    // Fetch raw records from Freebuff databases
    let raw_records = state.freebuff.fetch_all_records()?;

    // Normalize to UsageRecords
    let records: Vec<database::UsageRecord> = raw_records.iter()
        .map(freebuff_provider::normalize_record)
        .collect();

    // Persist to our database
    let (synced, total_records) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let count = db.upsert_usage_records(&records).map_err(|e| e.to_string())?;
        let total = db.count_records().map_err(|e| e.to_string())?;
        (count, total)
    };

    // Update sync timestamp
    if !records.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let now = Utc::now().timestamp_millis();
        db.update_sync_state("freebuff", now).map_err(|e| e.to_string())?;
    }

    info!("Freebuff sync complete: {} records upserted, {} total in database", synced, total_records);
    Ok(format!("Synced {} records ({} total)", synced, total_records))
}

#[tauri::command]
fn sync_all(state: State<'_, AppState>) -> Result<String, String> {
    let data_dir = get_data_dir();
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    let mut results = Vec::new();

    // Sync OpenCode
    match sync_opencode_inner(&state) {
        Ok(msg) => results.push(format!("OpenCode: {}", msg)),
        Err(e) => {
            error!("OpenCode sync failed: {}", e);
            results.push(format!("OpenCode: ERROR - {}", e));
        }
    }

    // Sync Freebuff
    match sync_freebuff_inner(&state) {
        Ok(msg) => results.push(format!("Freebuff: {}", msg)),
        Err(e) => {
            error!("Freebuff sync failed: {}", e);
            results.push(format!("Freebuff: ERROR - {}", e));
        }
    }

    Ok(results.join("\n"))
}

fn sync_opencode_inner(state: &AppState) -> Result<String, String> {
    let last_sync = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_last_sync_time("opencode").map_err(|e| e.to_string())?
    };

    let sessions = state.opencode.fetch_sessions(last_sync)?;

    let records: Vec<database::UsageRecord> = sessions.iter()
        .map(opencode_provider::OpenCodeProvider::normalize_session)
        .collect();

    let (synced, total_records) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let count = db.upsert_usage_records(&records).map_err(|e| e.to_string())?;
        let total = db.count_records().map_err(|e| e.to_string())?;
        (count, total)
    };

    if !records.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let now = Utc::now().timestamp_millis();
        db.update_sync_state("opencode", now).map_err(|e| e.to_string())?;
    }

    Ok(format!("Synced {} records ({} total)", synced, total_records))
}

fn sync_freebuff_inner(state: &AppState) -> Result<String, String> {
    let raw_records = state.freebuff.fetch_all_records()?;

    let records: Vec<database::UsageRecord> = raw_records.iter()
        .map(freebuff_provider::normalize_record)
        .collect();

    let (synced, total_records) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let count = db.upsert_usage_records(&records).map_err(|e| e.to_string())?;
        let total = db.count_records().map_err(|e| e.to_string())?;
        (count, total)
    };

    if !records.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let now = Utc::now().timestamp_millis();
        db.update_sync_state("freebuff", now).map_err(|e| e.to_string())?;
    }

    Ok(format!("Synced {} records ({} total)", synced, total_records))
}

#[tauri::command]
fn get_dashboard(state: State<'_, AppState>, days: Option<i64>, provider: Option<String>) -> Result<DashboardData, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    let summary = db.get_usage_summary(days, provider.as_deref()).map_err(|e| e.to_string())?;
    let provider_usage = db.get_provider_usage(days).map_err(|e| e.to_string())?;
    let model_usage = db.get_model_usage(days, provider.as_deref()).map_err(|e| e.to_string())?;
    let activity = db.get_activity(20, provider.as_deref()).map_err(|e| e.to_string())?;
    drop(db);

    let providers = build_provider_status(&state);

    Ok(DashboardData {
        summary,
        provider_usage,
        model_usage,
        activity,
        providers,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    let data_dir = get_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("Warning: Failed to create data directory {}: {}", data_dir.display(), e);
    }

    let db_path = data_dir.join("freetokenmeter.db");
    let db = Database::new(&db_path).expect("Failed to initialize database");

    let state = AppState {
        db: Mutex::new(db),
        opencode: opencode_provider::OpenCodeProvider::new(),
        freebuff: freebuff_provider::FreebuffProvider::new(),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            sync_opencode,
            sync_freebuff,
            sync_all,
            get_dashboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
