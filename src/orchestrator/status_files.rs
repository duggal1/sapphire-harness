use std::fs;
use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;
use serde_json::json;

use crate::protocol::StatusDirective;

pub struct StatusFileUpdate {
    pub modified_at: SystemTime,
    pub directive: StatusDirective,
    pub bootstrap: bool,
}

pub fn load_status_file_update(
    primary_path: &Path,
    hidden_path: &Path,
    previous_modified: Option<SystemTime>,
) -> Option<StatusFileUpdate> {
    let source_path = if primary_path.exists() {
        primary_path
    } else if hidden_path.exists() {
        hidden_path
    } else {
        return None;
    };

    let modified_at = fs::metadata(source_path).ok()?.modified().ok()?;
    if previous_modified.is_some_and(|previous| modified_at <= previous) {
        return None;
    }

    let content = fs::read_to_string(source_path).ok()?;
    let status_obj: serde_json::Value = serde_json::from_str(&content).ok()?;
    let state = status_obj.get("state")?.as_str()?.to_owned();
    let summary = status_obj
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    let files = collect_string_array(status_obj.get("files"));
    let commands = collect_string_array(status_obj.get("commands"));
    let risks = collect_string_array(status_obj.get("risks"));
    let overlap = status_obj
        .get("overlap")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let bootstrap = status_obj
        .get("bootstrap")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    Some(StatusFileUpdate {
        modified_at,
        directive: StatusDirective {
            state,
            summary,
            files,
            commands,
            risks,
            overlap,
        },
        bootstrap,
    })
}

pub fn write_bootstrap_status_files(
    primary_path: &Path,
    hidden_path: &Path,
    summary: &str,
) -> Result<()> {
    let payload = json!({
        "state": "progressing",
        "summary": summary,
        "files": [],
        "commands": [],
        "risks": [],
        "overlap": "none",
        "bootstrap": true,
    });
    let content = serde_json::to_string(&payload)?;
    write_status_file(primary_path, &content)?;
    write_status_file(hidden_path, &content)?;
    Ok(())
}

fn write_status_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn collect_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
