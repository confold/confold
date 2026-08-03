use super::preferences::{
    load_preferences, save_preferences, SemanticLanguage, SemanticPreferences,
};
use super::{
    apply_new_path, apply_to_paths, local_path, review_paths, SemanticReviewOutcome, SemanticTarget,
};
use crate::scan::FileRef;
use crate::sources::SourceSpec;
use confold_provider::{
    OpenAiCompatibleClient, ProviderError, ProviderProfile, StructuredCompletion,
    StructuredCompletionRequest, StructuredInferenceProvider,
};
use confold_semantic::{prepare, SemanticProposal, Verdict, SCHEMA_VERSION};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;

struct FakeProvider {
    content: String,
    calls: Cell<usize>,
    schema: RefCell<Option<Value>>,
}

fn provider_profile() -> ProviderProfile {
    ProviderProfile {
        model: "requested-model".to_string(),
        ..ProviderProfile::default()
    }
}

impl StructuredInferenceProvider for FakeProvider {
    fn complete_structured(
        &self,
        request: &StructuredCompletionRequest<'_>,
    ) -> Result<StructuredCompletion, ProviderError> {
        self.calls.set(self.calls.get() + 1);
        self.schema.replace(Some(request.schema.clone()));
        Ok(StructuredCompletion {
            model: "fixture-model".to_string(),
            content: self.content.clone(),
        })
    }
}

#[test]
fn review_paths_validates_the_provider_proposal_before_returning_it() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "# Plan\nLeft detail.\n").expect("write left");
    fs::write(&right, "# Plan\nRight detail.\n").expect("write right");
    let bundle = prepare(&left, &right, None).expect("prepare fixture bundle");
    let provider = FakeProvider {
        content: json!({
            "schema_version": SCHEMA_VERSION,
            "operation_id": bundle.operation_id,
            "verdict": "merged",
            "summary": "Both documents contribute distinct details.",
            "contributions": [
                { "source": "left", "intent": "Left detail.", "disposition": "preserved" },
                { "source": "right", "intent": "Right detail.", "disposition": "preserved" }
            ],
            "warnings": [],
            "result": "# Plan\nLeft detail.\nRight detail.\n"
        })
        .to_string(),
        calls: Cell::new(0),
        schema: RefCell::new(None),
    };

    let outcome = review_paths(
        &provider,
        &left,
        &right,
        &provider_profile(),
        SemanticLanguage::Spanish,
    )
    .expect("semantic review");
    let serialized = serde_json::to_value(&outcome).expect("serialize review outcome");
    assert_eq!(serialized["configuration"]["provider"], "ollama");
    assert_eq!(
        serialized["configuration"]["endpoint"],
        "http://127.0.0.1:11434/v1"
    );
    assert_eq!(
        serialized["configuration"]["requestedModel"],
        "requested-model"
    );
    assert_eq!(
        serialized["configuration"]["responseModel"],
        "fixture-model"
    );
    assert_eq!(serialized["configuration"]["language"], "es");

    let SemanticReviewOutcome::Review {
        model,
        proposal,
        report,
        ..
    } = outcome
    else {
        panic!("expected model-backed review");
    };
    assert_eq!(model, "fixture-model");
    assert_eq!(proposal.operation_id, report.operation_id);
    assert!(report.applicable);
    assert_eq!(report.diffs.len(), 2);
    assert_eq!(provider.calls.get(), 1);
    assert_eq!(
        provider.schema.borrow().as_ref().expect("captured schema")["properties"]["operation_id"]
            ["const"],
        report.operation_id
    );
}

#[test]
fn review_paths_skips_inference_for_identical_inputs() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "Same\n").expect("write left");
    fs::write(&right, "Same\n").expect("write right");
    let provider = FakeProvider {
        content: String::new(),
        calls: Cell::new(0),
        schema: RefCell::new(None),
    };

    let outcome = review_paths(
        &provider,
        &left,
        &right,
        &provider_profile(),
        SemanticLanguage::English,
    )
    .expect("deterministic review");

    assert!(matches!(
        outcome,
        SemanticReviewOutcome::FastPath {
            fast_path: confold_semantic::FastPath::ByteIdentical
        }
    ));
    assert_eq!(provider.calls.get(), 0);
}

#[test]
fn review_paths_rejects_a_proposal_for_another_operation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "Left\n").expect("write left");
    fs::write(&right, "Right\n").expect("write right");
    let provider = FakeProvider {
        content: json!({
            "schema_version": SCHEMA_VERSION,
            "operation_id": "another-operation",
            "verdict": "equivalent",
            "summary": "Equivalent.",
            "contributions": [
                { "source": "left", "intent": "Left.", "disposition": "already_present" },
                { "source": "right", "intent": "Right.", "disposition": "already_present" }
            ],
            "warnings": [],
            "result": null
        })
        .to_string(),
        calls: Cell::new(0),
        schema: RefCell::new(None),
    };

    let error = review_paths(
        &provider,
        &left,
        &right,
        &provider_profile(),
        SemanticLanguage::English,
    )
    .expect_err("mismatch must fail");

    assert!(error.contains("operation id mismatch"));
}

#[test]
fn local_path_accepts_filesystem_refs_and_rejects_remote_sources() {
    let local = FileRef {
        source: SourceSpec {
            kind: "fs".to_string(),
            fields: BTreeMap::from([("root".to_string(), "/tmp/docs".to_string())]),
        },
        rel: "guide.md".to_string(),
    };
    let remote = FileRef {
        source: SourceSpec {
            kind: "sftp".to_string(),
            fields: BTreeMap::new(),
        },
        rel: "guide.md".to_string(),
    };

    assert_eq!(
        local_path(&local).expect("local path"),
        std::path::PathBuf::from("/tmp/docs/guide.md")
    );
    assert!(local_path(&remote).is_err());
}

#[test]
#[ignore = "requires a local Ollama server and CONFOLD_OLLAMA_MODEL"]
fn live_ollama_produces_a_validated_semantic_review() {
    let model = std::env::var("CONFOLD_OLLAMA_MODEL").expect("CONFOLD_OLLAMA_MODEL");
    let profile = ProviderProfile {
        model,
        ..ProviderProfile::default()
    };
    let client = OpenAiCompatibleClient::new(profile.clone(), None, []).expect("build client");
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "# Guide\nShort introduction.\n").expect("write left");
    fs::write(&right, "# Guide\nDetailed example.\n").expect("write right");

    let outcome = review_paths(&client, &left, &right, &profile, SemanticLanguage::English)
        .expect("live semantic review");

    let SemanticReviewOutcome::Review { report, .. } = outcome else {
        panic!("expected model-backed review");
    };
    assert!(report.applicable);
    assert_eq!(report.diffs.len(), 2);
}

#[test]
fn semantic_preferences_round_trip_independently_of_provider_settings() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("semantic-settings.json");

    assert_eq!(
        load_preferences(&path).expect("default preferences"),
        SemanticPreferences::default()
    );
    save_preferences(
        &path,
        &SemanticPreferences {
            language: SemanticLanguage::Spanish,
        },
    )
    .expect("save preferences");

    assert_eq!(
        load_preferences(&path).expect("saved preferences").language,
        SemanticLanguage::Spanish
    );
}

#[test]
fn semantic_apply_paths_replace_only_the_explicit_side_or_create_a_new_file() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    let output = directory.path().join("merged.md");
    fs::write(&left, "left\n").expect("write left");
    fs::write(&right, "right\n").expect("write right");
    let bundle = prepare(&left, &right, None).expect("prepare bundle");
    let proposal = merged_proposal(&bundle.operation_id);

    apply_to_paths(&left, &right, &proposal, SemanticTarget::Destination)
        .expect("replace destination");
    assert_eq!(fs::read_to_string(&left).expect("read left"), "left\n");
    assert_eq!(fs::read_to_string(&right).expect("read right"), "merged\n");

    let refreshed = prepare(&left, &right, None).expect("prepare refreshed bundle");
    let refreshed_proposal = merged_proposal(&refreshed.operation_id);
    apply_new_path(&left, &right, &refreshed_proposal, &output).expect("create output");
    assert_eq!(
        fs::read_to_string(&output).expect("read output"),
        "merged\n"
    );
}

#[test]
fn semantic_apply_paths_reject_a_proposal_after_either_input_changes() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let left = directory.path().join("left.md");
    let right = directory.path().join("right.md");
    fs::write(&left, "left\n").expect("write left");
    fs::write(&right, "right\n").expect("write right");
    let bundle = prepare(&left, &right, None).expect("prepare bundle");
    let proposal = merged_proposal(&bundle.operation_id);
    fs::write(&left, "changed\n").expect("change left");

    let error = apply_to_paths(&left, &right, &proposal, SemanticTarget::Destination)
        .expect_err("stale proposal must fail");

    assert!(error.contains("operation id mismatch"));
    assert_eq!(fs::read_to_string(&right).expect("read right"), "right\n");
}

fn merged_proposal(operation_id: &str) -> SemanticProposal {
    SemanticProposal {
        schema_version: SCHEMA_VERSION,
        operation_id: operation_id.to_string(),
        verdict: Verdict::Merged,
        summary: "Merged fixture".to_string(),
        contributions: vec![
            confold_semantic::Contribution {
                source: confold_semantic::InputRole::Left,
                intent: "Left fixture".to_string(),
                disposition: confold_semantic::Disposition::Preserved,
            },
            confold_semantic::Contribution {
                source: confold_semantic::InputRole::Right,
                intent: "Right fixture".to_string(),
                disposition: confold_semantic::Disposition::Preserved,
            },
        ],
        warnings: Vec::new(),
        result: Some("merged\n".to_string()),
    }
}
