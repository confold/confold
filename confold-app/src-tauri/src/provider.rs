use confold_provider::{CapabilityReport, OpenAiCompatibleClient, ProviderProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::AppHandle;

mod profile;
mod secrets;

use profile::{clear_verification, load_verified_model, save_verification, verification_path};
use profile::{load_profile, normalize_profile, save_profile, settings_path};
use secrets::{
    load_client, persist_secrets, updated_secrets, NativeSecretStore, ProviderSecrets, SecretStore,
};
#[cfg(test)]
use {profile::SETTINGS_FILE, secrets::apply_secret_update};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSettingsResponse {
    profile: ProviderProfile,
    has_api_key: bool,
    has_custom_headers: bool,
    verified_model: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SaveProviderSettingsRequest {
    profile: ProviderProfile,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    clear_api_key: bool,
    #[serde(default)]
    custom_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    clear_custom_headers: bool,
}

fn response(
    profile: ProviderProfile,
    secrets: &ProviderSecrets,
    verified_model: Option<String>,
) -> ProviderSettingsResponse {
    ProviderSettingsResponse {
        profile,
        has_api_key: secrets.api_key.is_some(),
        has_custom_headers: !secrets.custom_headers.is_empty(),
        verified_model,
    }
}

pub(crate) fn load_verified_provider(
    app: &AppHandle,
) -> Result<(OpenAiCompatibleClient, ProviderProfile), String> {
    let path = settings_path(app)?;
    let profile = load_profile(&path)?;
    if load_verified_model(&verification_path(&path), &profile)?.is_none() {
        return Err(
            "test the active AI provider in Settings before starting semantic review".to_string(),
        );
    }
    let secrets = NativeSecretStore.load()?;
    let client =
        OpenAiCompatibleClient::new(profile.clone(), secrets.api_key, secrets.custom_headers)
            .map_err(|error| error.to_string())?;
    Ok((client, profile))
}

#[tauri::command]
pub(crate) fn load_provider_settings(app: AppHandle) -> Result<ProviderSettingsResponse, String> {
    let path = settings_path(&app)?;
    let profile = load_profile(&path)?;
    let secrets = NativeSecretStore.load()?;
    let verified_model = load_verified_model(&verification_path(&path), &profile)?;
    Ok(response(profile, &secrets, verified_model))
}

#[tauri::command]
pub(crate) fn save_provider_settings(
    app: AppHandle,
    request: SaveProviderSettingsRequest,
) -> Result<ProviderSettingsResponse, String> {
    let profile = normalize_profile(request.profile.clone())?;
    let store = NativeSecretStore;
    let secrets = updated_secrets(store.load()?, &request);
    OpenAiCompatibleClient::new(
        profile.clone(),
        secrets.api_key.clone(),
        secrets.custom_headers.clone(),
    )
    .map_err(|error| error.to_string())?;
    let path = settings_path(&app)?;
    persist_secrets(&store, &secrets)?;
    save_profile(&path, &profile)?;
    clear_verification(&verification_path(&path))?;
    Ok(response(profile, &secrets, None))
}

#[tauri::command]
pub(crate) async fn list_provider_models(app: AppHandle) -> Result<Vec<String>, String> {
    let path = settings_path(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        load_client(&path, &NativeSecretStore)?
            .list_models()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("provider discovery task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn test_provider_capability(app: AppHandle) -> Result<CapabilityReport, String> {
    let path = settings_path(&app)?;
    let verification = verification_path(&path);
    tauri::async_runtime::spawn_blocking(move || {
        let profile = load_profile(&path)?;
        let report = load_client(&path, &NativeSecretStore)?
            .test_capability()
            .map_err(|error| error.to_string())?;
        save_verification(&verification, &profile, &report.model)?;
        Ok(report)
    })
    .await
    .map_err(|error| format!("provider capability task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemorySecretStore {
        value: Mutex<Option<ProviderSecrets>>,
    }

    impl SecretStore for MemorySecretStore {
        fn load(&self) -> Result<ProviderSecrets, String> {
            Ok(self.value.lock().expect("lock").clone().unwrap_or_default())
        }

        fn save(&self, secrets: &ProviderSecrets) -> Result<(), String> {
            *self.value.lock().expect("lock") = Some(secrets.clone());
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            *self.value.lock().expect("lock") = None;
            Ok(())
        }
    }

    fn save_request(api_key: Option<&str>) -> SaveProviderSettingsRequest {
        SaveProviderSettingsRequest {
            profile: ProviderProfile {
                model: "model-a".to_string(),
                ..ProviderProfile::default()
            },
            api_key: api_key.map(str::to_string),
            clear_api_key: false,
            custom_headers: Some(BTreeMap::from([(
                "x-provider-route".to_string(),
                "private-route".to_string(),
            )])),
            clear_custom_headers: false,
        }
    }

    #[test]
    fn profile_file_never_contains_credentials_or_custom_headers() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join(SETTINGS_FILE);
        let request = save_request(Some("super-secret"));
        let store = MemorySecretStore::default();

        let secrets = apply_secret_update(&store, &request).expect("store secrets");
        save_profile(&path, &request.profile).expect("store profile");

        let serialized = std::fs::read_to_string(path).expect("read profile");
        assert!(!serialized.contains("super-secret"));
        assert!(!serialized.contains("private-route"));
        assert_eq!(secrets.api_key.as_deref(), Some("super-secret"));
        assert_eq!(
            secrets
                .custom_headers
                .get("x-provider-route")
                .map(String::as_str),
            Some("private-route")
        );
    }

    #[test]
    fn omitted_secret_fields_preserve_existing_keychain_values() {
        let store = MemorySecretStore::default();
        let initial = save_request(Some("first-key"));
        apply_secret_update(&store, &initial).expect("store initial secrets");
        let replacement = SaveProviderSettingsRequest {
            api_key: None,
            custom_headers: None,
            ..save_request(None)
        };

        let secrets = apply_secret_update(&store, &replacement).expect("preserve secrets");

        assert_eq!(secrets.api_key.as_deref(), Some("first-key"));
        assert_eq!(
            secrets
                .custom_headers
                .get("x-provider-route")
                .map(String::as_str),
            Some("private-route")
        );
    }

    #[test]
    fn explicit_clear_removes_all_keychain_values() {
        let store = MemorySecretStore::default();
        apply_secret_update(&store, &save_request(Some("first-key"))).expect("store secrets");
        let clear = SaveProviderSettingsRequest {
            api_key: None,
            clear_api_key: true,
            custom_headers: None,
            clear_custom_headers: true,
            ..save_request(None)
        };

        let secrets = apply_secret_update(&store, &clear).expect("clear secrets");

        assert_eq!(secrets, ProviderSecrets::default());
        assert!(store.value.lock().expect("lock").is_none());
    }
}
