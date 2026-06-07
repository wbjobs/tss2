pub mod db;
pub mod types;
pub mod wasm_runtime;
pub mod host_functions;
pub mod sandbox;
pub mod http_server;

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::db::Database;
use crate::wasm_runtime::WasmRuntimeManager;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub runtime_manager: Arc<Mutex<WasmRuntimeManager>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wasi_code_executor=debug,tower_http=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting WASI Code Executor Service...");

    let db = Arc::new(Database::new("executions.db").await?);
    db.init().await?;

    let runtime_manager = Arc::new(Mutex::new(WasmRuntimeManager::new()));

    let state = AppState {
        db: db.clone(),
        runtime_manager: runtime_manager.clone(),
    };

    let app = http_server::create_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("Server listening on http://0.0.0.0:8080");

    axum::serve(listener, app)
        .await?;

    Ok(())
}
