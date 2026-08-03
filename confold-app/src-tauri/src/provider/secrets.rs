use super::profile::load_profile;
use super::SaveProviderSettingsRequest;
use confold_provider::OpenAiCompatibleClient;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const KEYRING_SERVICE: &str = "com.confold.app";
const KEYRING_USER: &str = "ai-provider.active";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ProviderSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) api_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) custom_headers: BTreeMap<String, String>,
}

pub(super) trait SecretStore {
    fn load(&self) -> Result<ProviderSecrets, String>;
    fn save(&self, secrets: &ProviderSecrets) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

pub(super) struct NativeSecretStore;

impl NativeSecretStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| format!("could not open the operating-system keychain: {error}"))
    }
}

impl SecretStore for NativeSecretStore {
    fn load(&self) -> Result<ProviderSecrets, String> {
        match Self::entry()?.get_password() {
            Ok(serialized) => serde_json::from_str(&serialized)
                .map_err(|error| format!("stored provider credentials are invalid: {error}")),
            Err(keyring::Error::NoEntry) => Ok(ProviderSecrets::default()),
            Err(error) => Err(format!(
                "could not read provider credentials from the operating-system keychain: {error}"
            )),
        }
    }

    fn save(&self, secrets: &ProviderSecrets) -> Result<(), String> {
        let serialized = serde_json::to_string(secrets)
            .map_err(|error| format!("could not encode provider credentials: {error}"))?;
        Self::entry()?
            .set_password(&serialized)
            .map_err(|error| format!("could not save provider credentials: {error}"))
    }

    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("could not clear provider credentials: {error}")),
        }
    }
}

#[cfg(test)]
pub(super) fn apply_secret_update(
    store: &dyn SecretStore,
    request: &SaveProviderSettingsRequest,
) -> Result<ProviderSecrets, String> {
    let secrets = updated_secrets(store.load()?, request);
    persist_secrets(store, &secrets)?;
    Ok(secrets)
}

pub(super) fn updated_secrets(
    mut secrets: ProviderSecrets,
    request: &SaveProviderSettingsRequest,
) -> ProviderSecrets {
    if request.clear_api_key {
        secrets.api_key = None;
    } else if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        secrets.api_key = Some(api_key.to_string());
    }
    if request.clear_custom_headers {
        secrets.custom_headers.clear();
    } else if let Some(headers) = &request.custom_headers {
        secrets.custom_headers = headers
            .iter()
            .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
            .filter(|(name, _)| !name.is_empty())
            .collect();
    }
    secrets
}

pub(super) fn persist_secrets(
    store: &dyn SecretStore,
    secrets: &ProviderSecrets,
) -> Result<(), String> {
    if secrets.api_key.is_none() && secrets.custom_headers.is_empty() {
        store.delete()
    } else {
        store.save(secrets)
    }
}

pub(crate) fn load_client(
    path: &Path,
    store: &dyn SecretStore,
) -> Result<OpenAiCompatibleClient, String> {
    let profile = load_profile(path)?;
    let secrets = store.load()?;
    OpenAiCompatibleClient::new(profile, secrets.api_key, secrets.custom_headers)
        .map_err(|error| error.to_string())
}
