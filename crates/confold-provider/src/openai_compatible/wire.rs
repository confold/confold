use crate::{ProviderError, StructuredCompletionRequest};
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) fn structured_body(model: &str, request: &StructuredCompletionRequest<'_>) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": request.system_message },
            { "role": "user", "content": request.user_message }
        ],
        "stream": false,
        "temperature": 0,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": request.schema_name,
                "strict": true,
                "schema": request.schema
            }
        }
    })
}

pub(super) fn capability_body(model: &str) -> Value {
    json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": "Return only the requested JSON object with ok set to true."
        }],
        "stream": false,
        "temperature": 0,
        "max_tokens": 16,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "confold_capability_test",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "ok": { "type": "boolean", "const": true }
                    },
                    "required": ["ok"],
                    "additionalProperties": false
                }
            }
        }
    })
}

#[derive(Deserialize)]
pub(super) struct ModelsResponse {
    pub(super) data: Vec<Model>,
}

#[derive(Deserialize)]
pub(super) struct Model {
    pub(super) id: String,
}

#[derive(Deserialize)]
pub(super) struct ChatCompletion {
    pub(super) model: Option<String>,
    choices: Vec<Choice>,
}

impl ChatCompletion {
    pub(super) fn message(&self) -> Result<&Message, ProviderError> {
        self.choices
            .first()
            .map(|choice| &choice.message)
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "response did not contain an assistant message".to_string(),
                )
            })
    }
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
pub(super) struct Message {
    pub(super) content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapabilityProbe {
    pub(super) ok: bool,
}
