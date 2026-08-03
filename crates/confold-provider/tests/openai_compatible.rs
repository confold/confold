use confold_provider::{
    OpenAiCompatibleClient, ProviderKind, ProviderProfile, StructuredCompletionRequest,
    OLLAMA_DEFAULT_BASE_URL,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

struct MockServer {
    base_url: String,
    request: Receiver<String>,
    handle: JoinHandle<()>,
}

impl MockServer {
    fn respond(status: &str, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let address = listener.local_addr().expect("mock address");
        let (sender, request) = mpsc::channel();
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let captured = read_request(&mut stream);
            sender.send(captured).expect("send captured request");
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });
        Self {
            base_url: format!("http://{address}/v1"),
            request,
            handle,
        }
    }

    fn finish(self) -> String {
        let request = self
            .request
            .recv_timeout(Duration::from_secs(2))
            .expect("receive request");
        self.handle.join().expect("join mock server");
        request
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).expect("read request");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8_lossy(&bytes);
        let Some(header_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = text[..header_end]
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(bytes).expect("request is utf-8")
}

#[test]
fn ollama_profile_has_a_safe_local_default() {
    let profile = ProviderProfile::default();

    assert_eq!(profile.kind, ProviderKind::Ollama);
    assert_eq!(profile.base_url, OLLAMA_DEFAULT_BASE_URL);
    assert!(profile.model.is_empty());
    profile.validate().expect("default profile is valid");
}

#[test]
fn profile_rejects_non_http_urls_and_url_credentials() {
    let non_http = ProviderProfile {
        base_url: "file:///tmp/provider".to_string(),
        ..ProviderProfile::default()
    };
    assert!(non_http.validate().is_err());

    let credentials = ProviderProfile {
        base_url: "https://user:secret@example.com/v1".to_string(),
        ..ProviderProfile::default()
    };
    assert!(credentials.validate().is_err());
}

#[test]
fn model_discovery_uses_the_openai_models_contract_and_bearer_auth() {
    let server = MockServer::respond(
        "200 OK",
        r#"{"object":"list","data":[{"id":"small"},{"id":"large"}]}"#,
    );
    let profile = ProviderProfile {
        kind: ProviderKind::OpenAiCompatible,
        base_url: server.base_url.clone(),
        model: String::new(),
        use_api_key: true,
    };
    let client = OpenAiCompatibleClient::new(profile, Some("test-token".to_string()), [])
        .expect("build client");

    let models = client.list_models().expect("list models");

    assert_eq!(models, vec!["small", "large"]);
    let request = server.finish();
    assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-token\r\n"));
}

#[test]
fn capability_test_requires_schema_constrained_json() {
    let server = MockServer::respond(
        "200 OK",
        r#"{"model":"local-model:latest","choices":[{"message":{"role":"assistant","content":"{\"ok\":true}"}}]}"#,
    );
    let profile = ProviderProfile {
        kind: ProviderKind::Ollama,
        base_url: server.base_url.clone(),
        model: "local-model:latest".to_string(),
        use_api_key: false,
    };
    let client = OpenAiCompatibleClient::new(profile, None, []).expect("build client");

    let report = client.test_capability().expect("test capability");

    assert_eq!(report.model, "local-model:latest");
    let request = server.finish();
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    let body = request.split_once("\r\n\r\n").expect("request body").1;
    let json: Value = serde_json::from_str(body).expect("valid request JSON");
    assert_eq!(json["stream"], false);
    assert_eq!(json["response_format"]["type"], "json_schema");
    assert_eq!(
        json["response_format"]["json_schema"]["schema"]["additionalProperties"],
        false
    );
}

#[test]
fn capability_test_rejects_unstructured_or_wrong_json() {
    let server = MockServer::respond(
        "200 OK",
        r#"{"model":"bad-model","choices":[{"message":{"content":"{\"ok\":false}"}}]}"#,
    );
    let profile = ProviderProfile {
        kind: ProviderKind::OpenAiCompatible,
        base_url: server.base_url.clone(),
        model: "bad-model".to_string(),
        use_api_key: false,
    };
    let client = OpenAiCompatibleClient::new(profile, None, []).expect("build client");

    let error = client.test_capability().expect_err("wrong JSON must fail");

    assert!(error.to_string().contains("schema-constrained"));
    let _request = server.finish();
}

#[test]
fn structured_completion_forwards_the_schema_and_returns_assistant_content() {
    let server = MockServer::respond(
        "200 OK",
        r#"{"model":"semantic-model","choices":[{"message":{"content":"{\"verdict\":\"merged\"}"}}]}"#,
    );
    let profile = ProviderProfile {
        kind: ProviderKind::OpenAiCompatible,
        base_url: server.base_url.clone(),
        model: "configured-model".to_string(),
        use_api_key: false,
    };
    let client = OpenAiCompatibleClient::new(profile, None, []).expect("build client");
    let schema = json!({
        "type": "object",
        "properties": { "verdict": { "type": "string" } },
        "required": ["verdict"],
        "additionalProperties": false
    });
    let request = StructuredCompletionRequest {
        schema_name: "semantic_proposal",
        schema: &schema,
        system_message: "system",
        user_message: "user",
    };

    let completion = client
        .complete_structured(&request)
        .expect("complete structured request");

    assert_eq!(completion.model, "semantic-model");
    assert_eq!(completion.content, r#"{"verdict":"merged"}"#);
    let captured = server.finish();
    let body = captured
        .split_once("\r\n\r\n")
        .expect("captured request body")
        .1;
    let body: Value = serde_json::from_str(body).expect("valid request JSON");
    assert_eq!(body["model"], "configured-model");
    assert_eq!(body["stream"], false);
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        "semantic_proposal"
    );
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(body["response_format"]["json_schema"]["schema"], schema);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
}
