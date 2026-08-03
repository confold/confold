use confold_provider::{OpenAiCompatibleClient, ProviderProfile};

#[test]
#[ignore = "requires a local Ollama server and CONFOLD_OLLAMA_MODEL"]
fn live_ollama_honors_the_structured_output_contract() {
    let model = std::env::var("CONFOLD_OLLAMA_MODEL").expect("CONFOLD_OLLAMA_MODEL");
    let profile = ProviderProfile {
        model: model.clone(),
        ..ProviderProfile::default()
    };
    let client = OpenAiCompatibleClient::new(profile, None, []).expect("build client");

    let models = client.list_models().expect("discover Ollama models");
    assert!(models.contains(&model));
    let report = client.test_capability().expect("verify structured output");
    assert!(!report.model.is_empty());
}
