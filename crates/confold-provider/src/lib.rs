//! Provider-neutral model inference boundary for Confold.
//!
//! This crate owns remote model communication. It deliberately does not depend on
//! `confold-semantic`: provider responses must cross the semantic crate's existing validation
//! boundary before Confold can review or apply a proposal.

use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod openai_compatible;

pub use openai_compatible::OpenAiCompatibleClient;

/// Default OpenAI-compatible endpoint exposed by a local Ollama installation.
pub const OLLAMA_DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434/v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    Ollama,
    OpenAiCompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderProfile {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub use_api_key: bool,
}

impl Default for ProviderProfile {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Ollama,
            base_url: OLLAMA_DEFAULT_BASE_URL.to_string(),
            model: String::new(),
            use_api_key: false,
        }
    }
}

impl ProviderProfile {
    pub fn validate(&self) -> Result<(), ProviderError> {
        let url = Url::parse(self.base_url.trim())
            .map_err(|error| ProviderError::InvalidProfile(format!("invalid base URL: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ProviderError::InvalidProfile(
                "base URL must use http or https".to_string(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(ProviderError::InvalidProfile(
                "base URL must not contain credentials".to_string(),
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(ProviderError::InvalidProfile(
                "base URL must not contain a query or fragment".to_string(),
            ));
        }
        Ok(())
    }

    pub fn normalized(mut self) -> Result<Self, ProviderError> {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.model = self.model.trim().to_string();
        self.validate()?;
        Ok(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    pub model: String,
}

pub trait InferenceProvider {
    fn list_models(&self) -> Result<Vec<String>, ProviderError>;
    fn test_capability(&self) -> Result<CapabilityReport, ProviderError>;
}

pub struct StructuredCompletionRequest<'a> {
    pub schema_name: &'a str,
    pub schema: &'a Value,
    pub system_message: &'a str,
    pub user_message: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredCompletion {
    pub model: String,
    pub content: String,
}

pub trait StructuredInferenceProvider {
    fn complete_structured(
        &self,
        request: &StructuredCompletionRequest<'_>,
    ) -> Result<StructuredCompletion, ProviderError>;
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("invalid provider profile: {0}")]
    InvalidProfile(String),
    #[error("provider request failed: {0}")]
    Request(reqwest::Error),
    #[error("provider returned HTTP {status}: {message}")]
    Api { status: StatusCode, message: String },
    #[error("invalid provider response: {0}")]
    InvalidResponse(String),
}
