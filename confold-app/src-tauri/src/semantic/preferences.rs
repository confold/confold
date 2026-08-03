use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "semantic-settings.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) enum SemanticLanguage {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "es")]
    Spanish,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SemanticPreferences {
    pub(crate) language: SemanticLanguage,
}

pub(super) fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("could not locate the Confold configuration directory: {error}"))
}

pub(super) fn load_preferences(path: &Path) -> Result<SemanticPreferences, String> {
    match std::fs::read_to_string(path) {
        Ok(serialized) => Ok(serde_json::from_str(&serialized).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(SemanticPreferences::default())
        }
        Err(error) => Err(format!("could not read semantic settings: {error}")),
    }
}

pub(super) fn save_preferences(
    path: &Path,
    preferences: &SemanticPreferences,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "semantic settings path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!("could not create the Confold configuration directory: {error}")
    })?;
    let serialized = serde_json::to_string_pretty(preferences)
        .map_err(|error| format!("could not encode semantic settings: {error}"))?;
    std::fs::write(path, serialized)
        .map_err(|error| format!("could not save semantic settings: {error}"))
}
