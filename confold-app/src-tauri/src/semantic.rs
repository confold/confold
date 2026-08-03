use crate::provider::load_verified_provider;
use crate::scan::FileRef;
use confold_provider::{
    ProviderKind, ProviderProfile, StructuredCompletionRequest, StructuredInferenceProvider,
};
use confold_semantic::{
    apply, prepare, proposal_json_schema, replace_input, review, ApplyReport, FastPath,
    ReplacementTarget, ReviewReport, SemanticProposal,
};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

mod preferences;

use preferences::{
    load_preferences, preferences_path, save_preferences, SemanticLanguage, SemanticPreferences,
};

const ENGLISH_SYSTEM_MESSAGE: &str = "You reconcile prose documents for Confold. Compare intent and structure, preserve non-conflicting contributions, explain every omission or supersession, and return only the requested JSON proposal. Write the summary, contribution intents, and warnings in English. Preserve the source language, terminology, structure, and formatting in the proposed result; never translate the document. Use uncertain with no result when ambiguity cannot be resolved safely.";
const SPANISH_SYSTEM_MESSAGE: &str = "You reconcile prose documents for Confold. Compare intent and structure, preserve non-conflicting contributions, explain every omission or supersession, and return only the requested JSON proposal. Write the summary, contribution intents, and warnings in Spanish. Preserve the source language, terminology, structure, and formatting in the proposed result; never translate the document. Use uncertain with no result when ambiguity cannot be resolved safely.";

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SemanticReviewOutcome {
    FastPath {
        fast_path: FastPath,
    },
    Review {
        model: String,
        configuration: SemanticReviewConfiguration,
        origin_path: PathBuf,
        destination_path: PathBuf,
        proposal: Box<SemanticProposal>,
        report: Box<ReviewReport>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticReviewConfiguration {
    provider: ProviderKind,
    endpoint: String,
    requested_model: String,
    response_model: String,
    language: SemanticLanguage,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SemanticTarget {
    Origin,
    Destination,
}

fn system_message(language: SemanticLanguage) -> &'static str {
    match language {
        SemanticLanguage::English => ENGLISH_SYSTEM_MESSAGE,
        SemanticLanguage::Spanish => SPANISH_SYSTEM_MESSAGE,
    }
}

fn local_path(file: &FileRef) -> Result<PathBuf, String> {
    if file.source.kind != "fs" {
        return Err("semantic review currently supports local filesystem files only".to_string());
    }
    let relative = Path::new(&file.rel);
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("semantic file references must stay within their source root".to_string());
    }
    let root = file
        .source
        .fields
        .get("root")
        .ok_or_else(|| "local source is missing its root path".to_string())?;
    Ok(Path::new(root).join(relative))
}

pub(crate) fn review_paths(
    provider: &impl StructuredInferenceProvider,
    left: &Path,
    right: &Path,
    profile: &ProviderProfile,
    language: SemanticLanguage,
) -> Result<SemanticReviewOutcome, String> {
    let bundle = prepare(left, right, None).map_err(|error| error.to_string())?;
    if bundle.fast_path != FastPath::NeedsSemanticAnalysis {
        return Ok(SemanticReviewOutcome::FastPath {
            fast_path: bundle.fast_path,
        });
    }
    let schema = proposal_json_schema(&bundle);
    let serialized = serde_json::to_string(&bundle)
        .map_err(|error| format!("could not encode bundle: {error}"))?;
    let user_message = format!(
        "Analyze this validated SemanticBundle and produce its complete SemanticProposal:\n{serialized}"
    );
    let completion = provider
        .complete_structured(&StructuredCompletionRequest {
            schema_name: "confold_semantic_proposal",
            schema: &schema,
            system_message: system_message(language),
            user_message: &user_message,
        })
        .map_err(|error| error.to_string())?;
    let proposal = serde_json::from_str::<SemanticProposal>(&completion.content)
        .map_err(|error| format!("provider returned an invalid semantic proposal: {error}"))?;
    let report = review(&bundle, &proposal).map_err(|error| error.to_string())?;
    let origin_path = bundle
        .inputs
        .first()
        .ok_or_else(|| "semantic bundle has no origin input".to_string())?
        .path
        .clone();
    let destination_path = bundle
        .inputs
        .get(1)
        .ok_or_else(|| "semantic bundle has no destination input".to_string())?
        .path
        .clone();
    Ok(SemanticReviewOutcome::Review {
        model: completion.model.clone(),
        configuration: SemanticReviewConfiguration {
            provider: profile.kind,
            endpoint: profile.base_url.clone(),
            requested_model: profile.model.clone(),
            response_model: completion.model,
            language,
        },
        origin_path,
        destination_path,
        proposal: Box::new(proposal),
        report: Box::new(report),
    })
}

pub(crate) fn apply_to_paths(
    left: &Path,
    right: &Path,
    proposal: &SemanticProposal,
    target: SemanticTarget,
) -> Result<ApplyReport, String> {
    let bundle = prepare(left, right, None).map_err(|error| error.to_string())?;
    let target = match target {
        SemanticTarget::Origin => ReplacementTarget::Left,
        SemanticTarget::Destination => ReplacementTarget::Right,
    };
    replace_input(&bundle, proposal, target).map_err(|error| error.to_string())
}

pub(crate) fn apply_new_path(
    left: &Path,
    right: &Path,
    proposal: &SemanticProposal,
    output: &Path,
) -> Result<ApplyReport, String> {
    let bundle = prepare(left, right, None).map_err(|error| error.to_string())?;
    apply(&bundle, proposal, output).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn semantic_review(
    app: tauri::AppHandle,
    left: FileRef,
    right: FileRef,
) -> Result<SemanticReviewOutcome, String> {
    let left = local_path(&left)?;
    let right = local_path(&right)?;
    let language = load_preferences(&preferences_path(&app)?)?.language;
    tauri::async_runtime::spawn_blocking(move || {
        let (provider, profile) = load_verified_provider(&app)?;
        review_paths(&provider, &left, &right, &profile, language)
    })
    .await
    .map_err(|error| format!("semantic review task failed: {error}"))?
}

#[tauri::command]
pub(crate) fn load_semantic_preferences(
    app: tauri::AppHandle,
) -> Result<SemanticPreferences, String> {
    load_preferences(&preferences_path(&app)?)
}

#[tauri::command]
pub(crate) fn save_semantic_preferences(
    app: tauri::AppHandle,
    preferences: SemanticPreferences,
) -> Result<SemanticPreferences, String> {
    save_preferences(&preferences_path(&app)?, &preferences)?;
    Ok(preferences)
}

#[tauri::command]
pub(crate) async fn semantic_apply_to(
    left: FileRef,
    right: FileRef,
    proposal: SemanticProposal,
    target: SemanticTarget,
) -> Result<ApplyReport, String> {
    let left = local_path(&left)?;
    let right = local_path(&right)?;
    tauri::async_runtime::spawn_blocking(move || apply_to_paths(&left, &right, &proposal, target))
        .await
        .map_err(|error| format!("semantic apply task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn semantic_apply_new(
    left: FileRef,
    right: FileRef,
    proposal: SemanticProposal,
    output: PathBuf,
) -> Result<ApplyReport, String> {
    let left = local_path(&left)?;
    let right = local_path(&right)?;
    tauri::async_runtime::spawn_blocking(move || apply_new_path(&left, &right, &proposal, &output))
        .await
        .map_err(|error| format!("semantic apply task failed: {error}"))?
}

#[cfg(test)]
mod tests;
