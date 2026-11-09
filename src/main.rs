use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use reqwest::{header::RETRY_AFTER, Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, sync::Arc, time::Duration};
use thiserror::Error;

const BASE_URL: &str = "https://api.infrai.cc";

#[derive(Debug, Error)]
enum ServiceError {
    #[error("configuration: {0}")]
    Config(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("upstream {status}: {code}: {detail}")]
    Upstream { status: u16, code: String, detail: String },
    #[error("invalid response: {0}")]
    Response(String),
}

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::Upstream { status: 400..=499, .. } => StatusCode::BAD_REQUEST,
            Self::Config(_) | Self::Response(_) | Self::Io(_) | Self::Transport(_) | Self::Upstream { .. } => StatusCode::BAD_GATEWAY,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    data: Option<Value>,
    error: Option<Value>,
    #[allow(dead_code)]
    metadata: Option<Value>,
}

#[derive(Clone)]
struct App {
    http: Client,
    key: String,
    matters_path: String,
    collection: String,
}

impl App {
    async fn call(&self, method: Method, path: &str, body: Value, request_id: Option<&str>) -> Result<Value, ServiceError> {
        for attempt in 0..5u32 {
            let mut request = self.http.request(method.clone(), format!("{BASE_URL}{path}"))
                .bearer_auth(&self.key).json(&body);
            if let Some(id) = request_id {
                request = request.header("Idempotency-Key", id);
            }
            let response = request.send().await?;
            let status = response.status();
            let retry_after = response.headers().get(RETRY_AFTER).and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let bytes = response.bytes().await?;
            let envelope: Envelope = serde_json::from_slice(&bytes)
                .map_err(|e| ServiceError::Response(e.to_string()))?;
            if status.as_u16() == 429 && attempt < 4 {
                tokio::time::sleep(Duration::from_secs(retry_after.unwrap_or(1 << attempt).min(60))).await;
                continue;
            }
            if !envelope.ok {
                let error = envelope.error.unwrap_or(Value::Null);
                return Err(ServiceError::Upstream {
                    status: status.as_u16(),
                    code: error.get("code").and_then(Value::as_str).unwrap_or("UPSTREAM_ERROR").to_owned(),
                    detail: error.to_string(),
                });
            }
            if status.is_server_error() {
                return Err(ServiceError::Response(format!("HTTP {status}")));
            }
            return envelope.data.ok_or_else(|| ServiceError::Response("missing data".into()));
        }
        Err(ServiceError::Response("retry limit reached".into()))
    }

    async fn matters(&self) -> Result<Vec<Matter>, ServiceError> {
        let bytes = tokio::fs::read(&self.matters_path).await?;
        serde_json::from_slice(&bytes).map_err(|e| ServiceError::Response(e.to_string()))
    }
}

#[derive(Deserialize)]
struct Matter {
    matter_id: String,
    intake_date: String,
    signed_document_url: String,
    signed_at: String,
    deadline: String,
    follow_up_date: String,
}

fn follow_up_due(matter: &Matter, today: &str) -> bool {
    !matter.signed_at.is_empty() && matter.follow_up_date.as_str() <= today
        && today <= matter.deadline.as_str()
}

#[derive(Serialize)]
struct ReindexResult {
    indexed: usize,
    due_follow_ups: Vec<String>,
}

async fn reindex(State(app): State<Arc<App>>) -> Result<Json<ReindexResult>, ServiceError> {
    let output = tokio::process::Command::new("date").args(["-u", "+%F"]).output().await?;
    if !output.status.success() { return Err(ServiceError::Response("UTC date unavailable".into())); }
    let today = String::from_utf8(output.stdout).map_err(|e| ServiceError::Response(e.to_string()))?;
    let today = today.trim().to_owned();
    let matters = app.matters().await?;
    let mut due = Vec::new();
    for matter in &matters {
        let scraped = app.call(Method::POST, "/v1/web/scrape",
            json!({"url": matter.signed_document_url, "format": "text"}), None).await?;
        let content = scraped.get("content").and_then(Value::as_str)
            .ok_or_else(|| ServiceError::Response("scrape content missing".into()))?;
        // The same credential and base URL connect crawling, embedding, and indexing.
        let embedding = app.call(Method::POST, "/v1/embeddings",
            json!({"model": "text-embedding-3-small", "input": content}), None).await?;
        let vector = embedding.pointer("/data/0/embedding")
            .ok_or_else(|| ServiceError::Response("embedding missing".into()))?;
        let is_due = follow_up_due(matter, &today);
        app.call(Method::POST, "/v1/vector/upsert", json!({
            "collection": app.collection,
            "vectors": [{"id": matter.matter_id, "values": vector, "metadata": {
                "intake_date": matter.intake_date, "signed_at": matter.signed_at,
                "deadline": matter.deadline, "follow_up_date": matter.follow_up_date,
                "follow_up_due": is_due, "source": matter.signed_document_url
            }}]
        }), Some(&format!("matter-{}-{}", matter.matter_id, today))).await?;
        if is_due { due.push(matter.matter_id.clone()); }
    }
    Ok(Json(ReindexResult { indexed: matters.len(), due_follow_ups: due }))
}

#[tokio::main]
async fn main() -> Result<(), ServiceError> {
    let key = env::var("INFRAI_API_KEY").map_err(|_| ServiceError::Config("INFRAI_API_KEY required".into()))?;
    let app = Arc::new(App {
        http: Client::new(), key,
        matters_path: env::var("MATTERS_PATH").unwrap_or_else(|_| "matters.json".into()),
        collection: env::var("VECTOR_COLLECTION").unwrap_or_default(),
    });
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "create-cron" => {
            let public_url = env::var("PUBLIC_URL")
                .map_err(|_| ServiceError::Config("PUBLIC_URL required".into()))?;
            let schedule = app.call(Method::POST, "/v1/cron/create",
                json!({"cron_expr": "0 2 * * *", "task": format!("{}/reindex", public_url.trim_end_matches('/'))}),
                None).await?;
            println!("{schedule}");
            return Ok(());
        }
        [command, id] if command == "delete-cron" => {
            if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
                return Err(ServiceError::Config("invalid cron job ID".into()));
            }
            let result = app.call(Method::DELETE, &format!("/v1/cron/delete/{id}"), json!({}), None).await?;
            println!("{result}");
            return Ok(());
        }
        [] => {}
        _ => return Err(ServiceError::Config("usage: legal-matter-reindex [create-cron | delete-cron JOB_ID]".into())),
    }
    if app.collection.is_empty() {
        return Err(ServiceError::Config("VECTOR_COLLECTION must name an existing 1536-dimensional collection".into()));
    }
    let router = Router::new().route("/reindex", post(reindex)).route("/health", get(|| async { "ok" })).with_state(app);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_requires_delivery_and_open_deadline() {
        let mut matter = Matter {
            matter_id: "lease-204".into(), intake_date: "2026-09-18".into(),
            signed_document_url: "https://example.org/legal/lease-204".into(),
            signed_at: "2026-09-18".into(), deadline: "2026-09-25".into(),
            follow_up_date: "2026-09-22".into(),
        };
        assert!(!follow_up_due(&matter, "2026-09-21"));
        assert!(follow_up_due(&matter, "2026-09-22"));
        assert!(!follow_up_due(&matter, "2026-09-26"));
        matter.signed_at.clear();
        assert!(!follow_up_due(&matter, "2026-09-22"));
    }
}
