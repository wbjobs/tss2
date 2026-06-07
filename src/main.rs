pub mod cache;
pub mod db;
pub mod types;
pub mod wasm_runtime;
pub mod host_functions;
pub mod sandbox;
pub mod http_server;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::cache::{CacheStats, ExecutionCache};
use crate::db::Database;
use crate::wasm_runtime::WasmRuntimeManager;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub runtime_manager: Arc<Mutex<WasmRuntimeManager>>,
    pub execution_cache: Arc<ExecutionCache>,
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

    let cache_ttl = Duration::from_secs(
        std::env::var("CACHE_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300),
    );
    let cache_capacity = std::env::var("CACHE_CAPACITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let execution_cache = Arc::new(ExecutionCache::with_config(cache_capacity, cache_ttl));
    tracing::info!(
        "Execution cache initialized: capacity={}, ttl={}s",
        cache_capacity,
        cache_ttl.as_secs()
    );

    tracing::info!("Preloading Wasm modules for all languages...");
    let mut runtime_manager = WasmRuntimeManager::new();
    runtime_manager.preload_all_modules().await?;
    let warmup_stats = runtime_manager.get_warmup_stats();
    tracing::info!(
        "Module preload complete: {} modules loaded, took {:?}",
        warmup_stats.modules_loaded,
        warmup_stats.warmup_duration
    );

    let runtime_manager = Arc::new(Mutex::new(runtime_manager));

    let state = AppState {
        db: db.clone(),
        runtime_manager: runtime_manager.clone(),
        execution_cache: execution_cache.clone(),
    };

    spawn_cache_cleanup_task(execution_cache.clone());

    let app = http_server::create_router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("Server listening on http://0.0.0.0:8080");

    axum::serve(listener, app)
        .await?;

    Ok(())
}

fn spawn_cache_cleanup_task(cache: Arc<ExecutionCache>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let cleaned = cache.cleanup_expired();
            if cleaned > 0 {
                tracing::debug!("Cleaned up {} expired cache entries", cleaned);
            }
        }
    });
}
