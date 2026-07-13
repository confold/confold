use crate::shell::cache_dir;
use crate::sources::SourceSpec;
use std::path::Path;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredEntry {
    pub(crate) spec: SourceSpec,
    pub(crate) is_dir: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct StoredRecents {
    #[serde(default)]
    origins: Vec<StoredEntry>,
    #[serde(default)]
    destinations: Vec<StoredEntry>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentResponse {
    pub(crate) spec: SourceSpec,
    pub(crate) is_dir: bool,
    pub(crate) stale: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentsResponse {
    pub(crate) origins: Vec<RecentResponse>,
    pub(crate) destinations: Vec<RecentResponse>,
}

fn is_stale(spec: &SourceSpec) -> bool {
    if spec.kind == "fs" {
        spec.fields.get("root").map(|p| !Path::new(p).exists()).unwrap_or(false)
    } else {
        false
    }
}

#[tauri::command]
pub(crate) fn load_recents() -> RecentsResponse {
    let stored: StoredRecents = std::fs::read_to_string(cache_dir().join("recents.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let map = |e: &StoredEntry| RecentResponse {
        spec: e.spec.clone(),
        is_dir: e.is_dir,
        stale: is_stale(&e.spec),
    };

    RecentsResponse {
        origins: stored.origins.iter().map(map).collect(),
        destinations: stored.destinations.iter().map(map).collect(),
    }
}

#[tauri::command]
pub(crate) fn save_recents(origins: Vec<StoredEntry>, destinations: Vec<StoredEntry>) -> Result<(), String> {
    let data = StoredRecents { origins, destinations };
    let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(cache_dir()).map_err(|e| e.to_string())?;
    std::fs::write(cache_dir().join("recents.json"), json).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn path_exists(path: String) -> bool {
    Path::new(&path).exists()
}
