use super::{
    CapabilityReport, InferenceProvider, ProviderError, ProviderProfile, StructuredCompletion,
    StructuredCompletionRequest, StructuredInferenceProvider,
};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;

mod wire;

use wire::{capability_body, structured_body, CapabilityProbe, ChatCompletion, ModelsResponse};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const STRUCTURED_INFERENCE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_BODY_CHARS: usize = 500;

#[derive(Debug)]
pub struct OpenAiCompatibleClient {
    profile: ProviderProfile,
    client: Client,
    api_key: Option<String>,
    custom_headers: HeaderMap,
}

impl OpenAiCompatibleClient {
    pub fn new<I>(
        profile: ProviderProfile,
        api_key: Option<String>,
        custom_headers: I,
    ) -> Result<Self, ProviderError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let profile = profile.normalized()?;
        let api_key = api_key.filter(|value| !value.trim().is_empty());
        if profile.use_api_key && api_key.is_none() {
            return Err(ProviderError::InvalidProfile(
                "this profile requires an API key, but none is stored".to_string(),
            ));
        }

        let mut headers = HeaderMap::new();
        for (name, value) in custom_headers {
            let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|error| {
                ProviderError::InvalidProfile(format!("invalid custom header name: {error}"))
            })?;
            if name == AUTHORIZATION || name == CONTENT_TYPE {
                return Err(ProviderError::InvalidProfile(format!(
                    "custom header '{name}' is managed by Confold"
                )));
            }
            let value = HeaderValue::from_str(value.trim()).map_err(|error| {
                ProviderError::InvalidProfile(format!(
                    "invalid value for custom header '{name}': {error}"
                ))
            })?;
            headers.insert(name, value);
        }

        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("confold/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ProviderError::Request)?;
        Ok(Self {
            profile,
            client,
            api_key,
            custom_headers: headers,
        })
    }

    pub fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        <Self as InferenceProvider>::list_models(self)
    }

    pub fn test_capability(&self) -> Result<CapabilityReport, ProviderError> {
        <Self as InferenceProvider>::test_capability(self)
    }

    pub fn complete_structured(
        &self,
        request: &StructuredCompletionRequest<'_>,
    ) -> Result<StructuredCompletion, ProviderError> {
        <Self as StructuredInferenceProvider>::complete_structured(self, request)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.profile.base_url, path)
    }

    fn authorize(&self, mut request: RequestBuilder) -> RequestBuilder {
        request = request.headers(self.custom_headers.clone());
        if let Some(api_key) = &self.api_key {
            request = request.bearer_auth(api_key);
        }
        request
    }

    fn send(&self, request: RequestBuilder) -> Result<reqwest::blocking::Response, ProviderError> {
        let response = request.send().map_err(ProviderError::Request)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().unwrap_or_default();
        Err(ProviderError::Api {
            status,
            message: bounded_message(&body),
        })
    }
}

impl StructuredInferenceProvider for OpenAiCompatibleClient {
    fn complete_structured(
        &self,
        request: &StructuredCompletionRequest<'_>,
    ) -> Result<StructuredCompletion, ProviderError> {
        if self.profile.model.is_empty() {
            return Err(ProviderError::InvalidProfile(
                "select or enter a model before requesting inference".to_string(),
            ));
        }
        let completion = self.completion(
            structured_body(&self.profile.model, request),
            STRUCTURED_INFERENCE_TIMEOUT,
        )?;
        let content = completion.message()?.content.clone();
        Ok(StructuredCompletion {
            model: completion
                .model
                .unwrap_or_else(|| self.profile.model.clone()),
            content,
        })
    }
}

impl InferenceProvider for OpenAiCompatibleClient {
    fn list_models(&self) -> Result<Vec<String>, ProviderError> {
        let response = self.send(self.authorize(self.client.get(self.endpoint("models"))))?;
        let mut models = Vec::new();
        for model in response
            .json::<ModelsResponse>()
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?
            .data
        {
            let id = model.id.trim().to_string();
            if !id.is_empty() && !models.contains(&id) {
                models.push(id);
            }
        }
        Ok(models)
    }

    fn test_capability(&self) -> Result<CapabilityReport, ProviderError> {
        if self.profile.model.is_empty() {
            return Err(ProviderError::InvalidProfile(
                "select or enter a model before testing the provider".to_string(),
            ));
        }
        let completion = self.completion(capability_body(&self.profile.model), REQUEST_TIMEOUT)?;
        let content = completion.message()?.content.as_str();
        let probe = serde_json::from_str::<CapabilityProbe>(content).map_err(|error| {
            ProviderError::InvalidResponse(format!(
                "provider did not return schema-constrained JSON: {error}"
            ))
        })?;
        if !probe.ok {
            return Err(ProviderError::InvalidResponse(
                "provider did not return the required schema-constrained value".to_string(),
            ));
        }
        Ok(CapabilityReport {
            model: completion
                .model
                .unwrap_or_else(|| self.profile.model.clone()),
        })
    }
}

impl OpenAiCompatibleClient {
    fn completion(
        &self,
        body: serde_json::Value,
        timeout: Duration,
    ) -> Result<ChatCompletion, ProviderError> {
        self.send(
            self.authorize(
                self.client
                    .post(self.endpoint("chat/completions"))
                    .timeout(timeout)
                    .json(&body),
            ),
        )?
        .json()
        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))
    }
}

fn bounded_message(body: &str) -> String {
    let message = body.trim();
    if message.is_empty() {
        return "request rejected without an error message".to_string();
    }
    let mut chars = message.chars();
    let bounded = chars
        .by_ref()
        .take(MAX_ERROR_BODY_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}
