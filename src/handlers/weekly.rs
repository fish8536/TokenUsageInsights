use super::*;
use crate::db;
use crate::pricing::load_prepared_pricing_rules;
use crate::reporting::build_period_report;
use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};

/// API: 獲取可用的有使用記錄週次 (ISO 8601 YYYY-Www)
pub async fn get_available_weeks(Path(assistant): Path<String>) -> impl IntoResponse {
    let assistant = normalize_assistant_name(&assistant);
    if !is_supported_assistant_scope(&assistant) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "不支援的助理類型" })),
        )
            .into_response();
    }

    let res: Result<Vec<WeekInfo>, String> = tokio::task::spawn_blocking(move || {
        let conn = db::get_db_conn()?;
        db::get_available_weeks(&conn, &assistant)
    })
    .await
    .unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    match res {
        Ok(weeks) => Json(WeekListResponse { weeks }).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// API: 獲取指定週次的統計摘要數據
pub async fn get_weekly_details(
    Path((assistant, year_week)): Path<(String, String)>,
) -> impl IntoResponse {
    let assistant = normalize_assistant_name(&assistant);
    if !is_supported_assistant_scope(&assistant) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "不支援的助理類型" })),
        )
            .into_response();
    }

    let Some((start_date, end_date)) = db::parse_iso_week_range(&year_week) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "無效的週次格式，應為 YYYY-Www（例如 2026-W42）" })),
        )
            .into_response();
    };

    let report = tokio::task::spawn_blocking(move || {
        let conn = db::get_db_conn()?;
        let entries = db::get_usage_entries_by_date_range(&conn, &start_date, &end_date, &assistant)?;
        if entries.is_empty() {
            return Ok(None);
        }
        let pricing_rules = load_prepared_pricing_rules();
        let report = build_period_report(&entries, str::to_string, &pricing_rules);
        Ok::<_, String>(Some((year_week, start_date, end_date, report)))
    })
    .await
    .unwrap_or_else(|_| Err("執行緒執行失敗".to_string()));

    match report {
        Ok(Some((year_week, start_date, end_date, report))) => Json(WeeklyDetailsResponse {
            year_week,
            start_date,
            end_date,
            summary: report.summary,
            daily_breakdown: report
                .breakdown
                .into_iter()
                .map(|item| WeeklyDailyBreakdown {
                    date: item.label,
                    total_tokens: item.usage.total_tokens,
                    total_input_tokens: item.usage.input_tokens,
                    total_output_tokens: item.usage.output_tokens,
                    total_cache_read_tokens: item.usage.cache_read_tokens,
                    total_reasoning_tokens: item.usage.reasoning_tokens,
                    sessions_count: item.sessions_count,
                    cost_usd: item.usage.cost_usd,
                    agents: item.agents,
                })
                .collect(),
            projects: report.projects,
            models: report.models,
            agent_breakdown: report.agent_breakdown,
        })
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "找不到該週次的使用量資料。" })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_week_range_computes_monday_to_sunday() {
        let (start, end) = db::parse_iso_week_range("2026-W42").expect("valid week");
        assert_eq!(start, "2026-10-12"); // Monday
        assert_eq!(end, "2026-10-18");   // Sunday

        let (start1, end1) = db::parse_iso_week_range("2026-W01").expect("valid week 1");
        assert_eq!(start1, "2025-12-29");
        assert_eq!(end1, "2026-01-04");

        assert!(db::parse_iso_week_range("2026-W00").is_none());
        assert!(db::parse_iso_week_range("2026-W54").is_none());
        assert!(db::parse_iso_week_range("2026-42").is_none());
        assert!(db::parse_iso_week_range("invalid").is_none());
    }
}
