//! Config model + load/sanitize/save.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use uuid::Uuid;

// ══════════════════════════════════════════════════════════════════════════════
// Log syntax highlighting — span-based pipeline (inspired by tailspin)
// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
// Config — lives in the .json file the user passes
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvVar {
    pub key:   String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    #[serde(default)] pub id:   String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub executable:  String,
    #[serde(default)] pub working_dir: String,
    #[serde(default)] pub args:        String,
    #[serde(default)] pub log_path:    String,
    #[serde(default)] pub run_as_user: String,
    #[serde(default)] pub env_vars:    Vec<EnvVar>,
}

impl Component {
    pub(crate) fn new() -> Self {
        Self {
            id:           Uuid::new_v4().to_string(),
            name:         String::new(),
            executable:   String::new(),
            working_dir:  String::new(),
            args:         String::new(),
            log_path:     String::new(),
            run_as_user:  String::new(),
            env_vars:     vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    #[serde(default)] pub id:   String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub components: Vec<Component>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)] pub groups: Vec<Group>,
}

/// Load the config. A missing file is normal (fresh start); an unreadable or
/// malformed file is NOT silently replaced — the original is backed up first
/// and the error is surfaced so a later save can't destroy user data.
pub(crate) fn load_config(path: &Path) -> (AppConfig, Option<String>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (AppConfig::default(), None);
        }
        Err(e) => {
            return (AppConfig::default(), Some(format!(
                "Could not read {}: {}. Starting empty — saving will overwrite it.",
                path.display(), e
            )));
        }
    };
    match serde_json::from_str::<AppConfig>(&raw) {
        Ok(c)  => (c, None),
        Err(e) => {
            let backup = path.with_extension("json.corrupt");
            let note = match std::fs::copy(path, &backup) {
                Ok(_)   => format!("Original backed up to {}.", backup.display()),
                Err(be) => format!("Backup failed: {}.", be),
            };
            (AppConfig::default(), Some(format!(
                "Config {} is not valid JSON ({}). {} Starting empty.",
                path.display(), e, note
            )))
        }
    }
}

/// Repair missing/duplicate ids (e.g. hand-edited config). Returns true if
/// anything changed so the caller can mark the config dirty.
pub(crate) fn sanitize_config(config: &mut AppConfig) -> bool {
    let mut changed = false;
    let mut seen: HashSet<String> = HashSet::new();
    for g in &mut config.groups {
        if g.id.is_empty() || !seen.insert(g.id.clone()) {
            g.id = Uuid::new_v4().to_string();
            seen.insert(g.id.clone());
            changed = true;
        }
        for c in &mut g.components {
            if c.id.is_empty() || !seen.insert(c.id.clone()) {
                c.id = Uuid::new_v4().to_string();
                seen.insert(c.id.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Atomic save: write to a temp file, then rename over the target — a crash
/// mid-write can never leave a truncated config behind. Errors propagate so
/// the caller can keep the dirty flag set and tell the user.
pub(crate) fn save_config(config: &AppConfig, path: &Path) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() { std::fs::create_dir_all(p)?; }
    }
    let s = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, s)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
