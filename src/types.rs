use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    Ruby,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::Ruby => "ruby",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "python" | "py" => Some(Language::Python),
            "javascript" | "js" => Some(Language::JavaScript),
            "ruby" | "rb" => Some(Language::Ruby),
            _ => None,
        }
    }

    pub fn wasm_module_name(&self) -> &'static str {
        match self {
            Language::Python => "python-wasi",
            Language::JavaScript => "quickjs-wasi",
            Language::Ruby => "ruby-wasi",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub language: Language,
    pub code: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub use_cache: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExecuteResponse {
    pub execution_id: Uuid,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: u64,
    pub error: Option<String>,
    #[serde(default)]
    pub cached: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatsResponse {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub average_execution_time_ms: f64,
    pub by_language: Vec<LanguageStats>,
    pub recent_executions: Vec<ExecutionRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LanguageStats {
    pub language: Language,
    pub count: u64,
    pub success_count: u64,
    pub average_time_ms: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExecutionRecord {
    pub id: Uuid,
    pub language: Language,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: u64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SandboxLimits {
    pub max_memory_pages: u32,
    pub max_execution_time_ms: u64,
    pub allow_network: bool,
    pub allowed_domains: Vec<String>,
    pub allow_file_write: bool,
    pub sandbox_dir: String,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            max_memory_pages: 256,
            max_execution_time_ms: 5000,
            allow_network: false,
            allowed_domains: vec![],
            allow_file_write: false,
            sandbox_dir: "./sandbox".to_string(),
        }
    }
}
