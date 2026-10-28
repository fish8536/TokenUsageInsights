//! Antigravity CLI's native, per-response token records. No statusline required.
use crate::db::{TokenStats, UsageEntry};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub(crate) const SOURCE_KIND: &str = "antigravity-session";
// Bump when parsed fields change so unchanged historical sources are re-imported.
pub(crate) const PARSER_VERSION: u32 = 3;

pub(crate) fn find_session_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let brain = root.join("brain");
    if !brain.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(&brain).map_err(|e| format!("讀取 Antigravity brain 失敗: {e}"))?
    {
        let entry = entry.map_err(|e| format!("讀取 Antigravity session 失敗: {e}"))?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let path = entry
            .path()
            .join(".system_generated/logs/transcript_full.jsonl");
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn session_id(path: &Path) -> Option<&str> {
    path.ancestors().nth(3)?.file_name()?.to_str()
}

/// Only complete JSONL lines are consumed. A malformed completed record is an
/// error so a partially rebuilt session never replaces previously known usage.
pub(crate) fn parse_session(path: &Path) -> Result<Vec<UsageEntry>, String> {
    let session_id = session_id(path).ok_or("無效的 Antigravity transcript 路徑")?;
    let metadata = path
        .ancestors()
        .nth(5)
        .map(|root| crate::antigravity_metadata::read_session_metadata(root, session_id))
        .transpose()
        .map_err(|e| format!("讀取 Antigravity 會話 metadata 失敗: {e}"))?
        .unwrap_or_default();
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut line = String::new();
    let mut turn_no = 0;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(_) => continue,
        };
        if event.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
            continue;
        }
        turn_no += 1;
        if event.get("input_tokens").is_none()
            && event.get("output_tokens").is_none()
            && event.get("cache_read_tokens").is_none()
        {
            continue;
        }
        let count = |key: &str| -> Result<u64, String> {
            match event.get(key) {
                None | Some(Value::Null) => Ok(0),
                Some(value) => value
                    .as_u64()
                    .filter(|v| *v <= i64::MAX as u64)
                    .ok_or_else(|| format!("Antigravity {key} 必須是非負整數")),
            }
        };
        // Antigravity stores uncached input separately from cache reads.
        let input = count("input_tokens")?;
        let output = count("output_tokens")?;
        let cache_read = count("cache_read_tokens")?;
        let cache_write = count("cache_write_tokens")?;
        let total = input
            .checked_add(output)
            .and_then(|v| v.checked_add(cache_read))
            .and_then(|v| v.checked_add(cache_write))
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or("Antigravity token 總數超出範圍")?;
        let timestamp = event
            .get("created_at")
            .and_then(Value::as_str)
            .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
            .ok_or("Antigravity 回應缺少有效的 created_at")?
            .to_owned();
        let model = event
            .get("model_id")
            .or_else(|| event.get("model"))
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| {
                event
                    .get("step_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index).ok())
                    .and_then(|index| metadata.models.get(&index).cloned())
            });
        let tokens = TokenStats {
            input,
            output,
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
            cache_write_5m: None,
            cache_write_1h: None,
            reasoning: None,
            total,
        };
        entries.push(UsageEntry {
            timestamp,
            session_id: session_id.to_owned(),
            session_name: None,
            transcript_path: Some(path.to_string_lossy().into_owned()),
            cwd: metadata.cwd.clone(),
            version: None,
            turn_no,
            model: model.clone(),
            model_id: model,
            tokens: Some(tokens.clone()),
            delta_tokens: Some(tokens),
            context: None,
            cost: None,
            source_kind: Some(SOURCE_KIND.to_owned()),
            source_dir_key: None,
            parent_session_id: None,
            agent_nickname: None,
            agent_role: None,
            reasoning_effort: None,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "token-usage-insights-antigravity-test-{}-{}",
            label,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parse_session_skips_malformed_json_lines_and_extracts_valid_usage() {
        let root = temp_test_dir("malformed_json");
        let session_dir = root
            .join("brain")
            .join("test-session-1")
            .join(".system_generated/logs");
        fs::create_dir_all(&session_dir).unwrap();
        let transcript = session_dir.join("transcript_full.jsonl");

        let content = concat!(
            "{\"step_index\":0,\"source\":\"USER_EXPLICIT\",\"type\":\"USER_INPUT\",\"created_at\":\"2026-06-29T07:23:22Z\"}\n",
            "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-06-29T07:23:23Z\",\"input_tokens\":100,\"output_tokens\":20,\"cache_read_tokens\":50,\"model\":\"gemini-2.5-pro\"}\n",
            "s been modified to include a line number before every line, in the format: <line_number>: <original_line>\n",
            "{\"step_index\":3,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-06-29T07:23:25Z\",\"input_tokens\":200,\"output_tokens\":40,\"cache_read_tokens\":0,\"model\":\"gemini-2.5-pro\"}\n"
        );
        fs::write(&transcript, content).unwrap();

        let entries = parse_session(&transcript)
            .expect("parse_session should succeed despite malformed lines");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].session_id, "test-session-1");
        assert_eq!(entries[0].turn_no, 1);
        assert_eq!(entries[0].tokens.as_ref().unwrap().input, 100);
        assert_eq!(entries[0].tokens.as_ref().unwrap().output, 20);
        assert_eq!(entries[0].tokens.as_ref().unwrap().cache_read, Some(50));
        assert_eq!(entries[0].tokens.as_ref().unwrap().total, 170);

        assert_eq!(entries[1].turn_no, 2);
        assert_eq!(entries[1].tokens.as_ref().unwrap().input, 200);
        assert_eq!(entries[1].tokens.as_ref().unwrap().output, 40);
        assert_eq!(entries[1].tokens.as_ref().unwrap().total, 240);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_session_skips_unclosed_trailing_line() {
        let root = temp_test_dir("unclosed_line");
        let session_dir = root
            .join("brain")
            .join("test-session-2")
            .join(".system_generated/logs");
        fs::create_dir_all(&session_dir).unwrap();
        let transcript = session_dir.join("transcript_full.jsonl");

        let mut file = File::create(&transcript).unwrap();
        writeln!(
            file,
            "{{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-06-29T07:23:23Z\",\"input_tokens\":50,\"output_tokens\":10,\"model\":\"gemini-2.5-pro\"}}"
        ).unwrap();
        // Trailing unclosed line without newline
        write!(
            file,
            "{{\"step_index\":2,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-06-29T07:23:24Z\",\"input_tokens\":60"
        ).unwrap();

        let entries =
            parse_session(&transcript).expect("parse_session should ignore trailing partial line");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tokens.as_ref().unwrap().input, 50);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_session_fails_on_invalid_token_values() {
        let root = temp_test_dir("invalid_tokens");
        let session_dir = root
            .join("brain")
            .join("test-session-3")
            .join(".system_generated/logs");
        fs::create_dir_all(&session_dir).unwrap();
        let transcript = session_dir.join("transcript_full.jsonl");

        fs::write(
            &transcript,
            "{\"step_index\":1,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"created_at\":\"2026-06-29T07:23:23Z\",\"input_tokens\":-50,\"output_tokens\":10}\n",
        ).unwrap();

        let err = parse_session(&transcript).unwrap_err();
        assert!(err.contains("必須是非負整數"));

        let _ = fs::remove_dir_all(&root);
    }
}
