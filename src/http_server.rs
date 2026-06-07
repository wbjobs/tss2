use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, error};

use crate::AppState;
use crate::types::{ExecuteRequest, ExecuteResponse, StatsResponse};

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/execute", post(execute_handler))
        .route("/stats", get(stats_handler))
        .route("/health", get(health_handler))
        .route("/functions", get(functions_handler))
        .with_state(state)
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorResponse {
    error: String,
}

async fn execute_handler(
    State(state): State<AppState>,
    Json(request): Json<ExecuteRequest>,
) -> Response {
    debug!(
        "Received execute request: language={}, code_len={}",
        request.language.as_str(),
        request.code.len()
    );

    if request.code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Code cannot be empty".to_string(),
            }),
        )
            .into_response();
    }

    if request.code.len() > 100_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Code too large (max 100KB)".to_string(),
            }),
        )
            .into_response();
    }

    let runtime_manager = state.runtime_manager.lock().await;
    let result = runtime_manager
        .execute_code(request.language, &request.code, request.timeout_ms)
        .await;

    match result {
        Ok(response) => {
            if let Err(e) = state
                .db
                .record_execution(
                    response.execution_id,
                    request.language,
                    response.success,
                    &response.stdout,
                    &response.stderr,
                    response.execution_time_ms,
                    response.error.as_deref(),
                    &request.code,
                )
                .await
            {
                error!("Failed to record execution: {}", e);
            }

            info!(
                "Execution completed: id={}, success={}, time={}ms",
                response.execution_id, response.success, response.execution_time_ms
            );

            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("Execution failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Execution failed: {}", e),
                }),
            )
                .into_response()
        }
    }
}

async fn stats_handler(State(state): State<AppState>) -> Response {
    debug!("Received stats request");

    match state.db.get_stats().await {
        Ok(stats) => {
            debug!("Stats retrieved successfully");
            (StatusCode::OK, Json(stats)).into_response()
        }
        Err(e) => {
            error!("Failed to get stats: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get stats: {}", e),
                }),
            )
                .into_response()
        }
    }
}

async fn health_handler() -> Response {
    #[derive(Serialize)]
    struct HealthResponse {
        status: &'static str,
        version: &'static str,
    }

    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "healthy",
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
        .into_response()
}

async fn functions_handler(State(state): State<AppState>) -> Response {
    debug!("Received functions request");

    let runtime_manager = state.runtime_manager.lock().await;
    let functions = runtime_manager.get_available_host_functions();

    #[derive(Serialize)]
    struct FunctionsResponse {
        functions: Vec<String>,
    }

    (
        StatusCode::OK,
        Json(FunctionsResponse { functions }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::wasm_runtime::WasmRuntimeManager;
    use crate::types::Language;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn create_test_state() -> AppState {
        let db = Arc::new(Database::new(":memory:").await.unwrap());
        db.init().await.unwrap();

        let runtime_manager = Arc::new(Mutex::new(WasmRuntimeManager::new()));

        AppState {
            db,
            runtime_manager,
        }
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .get(&format!("http://{}/health", addr))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["status"], "healthy");
    }

    #[tokio::test]
    async fn test_execute_python() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::Python,
                code: "print('Hello from Python!')".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: ExecuteResponse = response.json().await.unwrap();
        assert!(body.success);
        assert!(body.stdout.contains("Hello from Python!"));
    }

    #[tokio::test]
    async fn test_execute_javascript() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::JavaScript,
                code: "console.log('Hello from JavaScript!');".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: ExecuteResponse = response.json().await.unwrap();
        assert!(body.success);
        assert!(body.stdout.contains("Hello from JavaScript!"));
    }

    #[tokio::test]
    async fn test_execute_ruby() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::Ruby,
                code: "puts 'Hello from Ruby!'".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: ExecuteResponse = response.json().await.unwrap();
        assert!(body.success);
        assert!(body.stdout.contains("Hello from Ruby!"));
    }

    #[tokio::test]
    async fn test_cross_language_function_call() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::Python,
                code: "print('fib(10) =', fibonacci(10))\nprint('2 + 3 =', add(2, 3))\nprint('is_prime(17) =', is_prime(17))".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: ExecuteResponse = response.json().await.unwrap();
        assert!(body.success);
        assert!(body.stdout.contains("fib(10)"));
        assert!(body.stdout.contains("2 + 3"));
        assert!(body.stdout.contains("is_prime(17)"));
    }

    #[tokio::test]
    async fn test_stats_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let _ = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::Python,
                code: "print('test')".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        let response = client
            .get(&format!("http://{}/stats", addr))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: StatsResponse = response.json().await.unwrap();
        assert_eq!(body.total_executions, 1);
        assert_eq!(body.successful_executions, 1);
        assert!(!body.by_language.is_empty());
    }

    #[tokio::test]
    async fn test_empty_code() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&format!("http://{}/execute", addr))
            .json(&ExecuteRequest {
                language: Language::Python,
                code: "".to_string(),
                timeout_ms: Some(3000),
            })
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_functions_endpoint() {
        let state = create_test_state().await;
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let response = client
            .get(&format!("http://{}/functions", addr))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body: serde_json::Value = response.json().await.unwrap();
        let functions = body["functions"].as_array().unwrap();
        assert!(!functions.is_empty());
    }
}
