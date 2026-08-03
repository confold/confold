use confold_provider::{ProviderKind, ProviderProfile, OLLAMA_DEFAULT_BASE_URL};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub(super) const SETTINGS_FILE: &str = "ai-provider.json";
const VERIFICATION_FILE: &str = "ai-provider-verification.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderVerification {
    profile: ProviderProfile,
    model: String,
}

pub(super) fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("could not locate the Confold configuration directory: {error}"))
}

pub(super) fn verification_path(settings: &Path) -> PathBuf {
    settings.with_file_name(VERIFICATION_FILE)
}

pub(super) fn load_profile(path: &Path) -> Result<ProviderProfile, String> {
    let profile = match std::fs::read_to_string(path) {
        Ok(serialized) => serde_json::from_str(&serialized)
            .map_err(|error| format!("provider settings are invalid: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ProviderProfile::default(),
        Err(error) => return Err(format!("could not read provider settings: {error}")),
    };
    normalize_profile(profile)
}

pub(super) fn save_profile(path: &Path, profile: &ProviderProfile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "provider settings path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!("could not create the Confold configuration directory: {error}")
    })?;
    let serialized = serde_json::to_string_pretty(profile)
        .map_err(|error| format!("could not encode provider settings: {error}"))?;
    std::fs::write(path, serialized)
        .map_err(|error| format!("could not save provider settings: {error}"))
}

pub(super) fn normalize_profile(mut profile: ProviderProfile) -> Result<ProviderProfile, String> {
    if profile.kind == ProviderKind::Ollama && profile.base_url.trim().is_empty() {
        profile.base_url = OLLAMA_DEFAULT_BASE_URL.to_string();
    }
    profile.normalized().map_err(|error| error.to_string())
}

pub(super) fn load_verified_model(
    path: &Path,
    profile: &ProviderProfile,
) -> Result<Option<String>, String> {
    let verification = match std::fs::read_to_string(path) {
        Ok(serialized) => serde_json::from_str::<ProviderVerification>(&serialized)
            .map_err(|error| format!("provider verification is invalid: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read provider verification: {error}")),
    };
    Ok((verification.profile == *profile).then_some(verification.model))
}

pub(super) fn save_verification(
    path: &Path,
    profile: &ProviderProfile,
    model: &str,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(&ProviderVerification {
        profile: profile.clone(),
        model: model.to_string(),
    })
    .map_err(|error| format!("could not encode provider verification: {error}"))?;
    std::fs::write(path, serialized)
        .map_err(|error| format!("could not save provider verification: {error}"))
}

pub(super) fn clear_verification(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not clear provider verification: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_verification, load_verified_model, save_verification};
    use confold_provider::ProviderProfile;

    #[test]
    fn verification_is_bound_to_the_exact_provider_profile() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("verification.json");
        let profile = ProviderProfile {
            model: "model-a".to_string(),
            ..ProviderProfile::default()
        };
        save_verification(&path, &profile, "model-a").expect("save verification");

        assert_eq!(
            load_verified_model(&path, &profile).expect("load verification"),
            Some("model-a".to_string())
        );
        let changed = ProviderProfile {
            model: "model-b".to_string(),
            ..profile
        };
        assert_eq!(
            load_verified_model(&path, &changed).expect("load stale verification"),
            None
        );
    }

    #[test]
    fn clearing_verification_accepts_an_absent_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("verification.json");

        clear_verification(&path).expect("clear absent verification");

        assert!(!path.exists());
    }
}
