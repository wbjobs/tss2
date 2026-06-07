use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use wasmtime_wasi::{
    WasiCtxBuilder,
};
use tokio::fs;

use crate::types::SandboxLimits;

#[derive(Clone)]
pub struct SandboxConfig {
    limits: SandboxLimits,
    allowed_paths: Arc<HashSet<PathBuf>>,
    blocked_paths: Arc<HashSet<PathBuf>>,
}

impl SandboxConfig {
    pub fn new(limits: SandboxLimits) -> Result<Self> {
        let mut allowed_paths = HashSet::new();
        let mut blocked_paths = HashSet::new();

        blocked_paths.insert(PathBuf::from("/"));
        blocked_paths.insert(PathBuf::from("/etc"));
        blocked_paths.insert(PathBuf::from("/root"));
        blocked_paths.insert(PathBuf::from("/home"));
        blocked_paths.insert(PathBuf::from("/var"));

        let sandbox_dir = PathBuf::from(&limits.sandbox_dir);
        allowed_paths.insert(sandbox_dir.clone());

        Ok(Self {
            limits,
            allowed_paths: Arc::new(allowed_paths),
            blocked_paths: Arc::new(blocked_paths),
        })
    }

    pub async fn prepare_sandbox_dir(&self, execution_id: &str) -> Result<PathBuf> {
        let sandbox_root = PathBuf::from(&self.limits.sandbox_dir);
        let exec_dir = sandbox_root.join(execution_id);

        fs::create_dir_all(&exec_dir).await?;

        let readme = exec_dir.join("README.txt");
        fs::write(
            &readme,
            format!(
                "Sandbox directory for execution: {}\n\
                This directory is isolated for this execution only.\n\
                Network access: {}\n\
                File write access: {}\n",
                execution_id,
                if self.limits.allow_network { "allowed" } else { "denied" },
                if self.limits.allow_file_write { "allowed" } else { "read-only" }
            ),
        )
        .await?;

        Ok(exec_dir)
    }

    pub async fn cleanup_sandbox_dir(&self, execution_id: &str) -> Result<()> {
        let sandbox_root = PathBuf::from(&self.limits.sandbox_dir);
        let exec_dir = sandbox_root.join(execution_id);

        if exec_dir.exists() {
            fs::remove_dir_all(&exec_dir).await?;
        }

        Ok(())
    }

    pub fn is_path_allowed(&self, path: &Path) -> bool {
        let canonical_path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };

        for blocked in self.blocked_paths.iter() {
            if canonical_path.starts_with(blocked) {
                let mut has_override = false;
                for allowed in self.allowed_paths.iter() {
                    if canonical_path.starts_with(allowed) {
                        has_override = true;
                        break;
                    }
                }
                if !has_override {
                    return false;
                }
            }
        }

        for allowed in self.allowed_paths.iter() {
            if canonical_path.starts_with(allowed) {
                return true;
            }
        }

        false
    }

    pub fn is_domain_allowed(&self, domain: &str) -> bool {
        if !self.limits.allow_network {
            return false;
        }

        if self.limits.allowed_domains.is_empty() {
            return true;
        }

        self.limits
            .allowed_domains
            .iter()
            .any(|d| domain == d || domain.ends_with(&format!(".{}", d)))
    }

    pub fn build_wasi_ctx(
        &self,
        exec_dir: &Path,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        stdin: Vec<u8>,
    ) -> Result<wasmtime_wasi::WasiCtx> {
        let mut builder = WasiCtxBuilder::new();

        let stdout_cursor = Cursor::new(stdout);
        let stderr_cursor = Cursor::new(stderr);
        let stdin_cursor = Cursor::new(stdin);

        builder.stdout(Box::new(stdout_cursor));
        builder.stderr(Box::new(stderr_cursor));
        builder.stdin(Box::new(stdin_cursor));

        builder.preopened_dir(exec_dir, "/sandbox")?;

        let tmp_dir = std::env::temp_dir().join("wasi-sandbox-tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        builder.preopened_dir(tmp_dir, "/tmp")?;

        builder.env("RUNTIME", "wasi-sandbox")?;
        builder.env("SANDBOX", "true")?;
        builder.env("HOME", "/sandbox")?;
        builder.env("TMPDIR", "/tmp")?;

        let max_memory = self.limits.max_memory_pages * 64 * 1024;
        builder.env("MAX_MEMORY_BYTES", &max_memory.to_string())?;

        Ok(builder.build())
    }

    pub fn limits(&self) -> &SandboxLimits {
        &self.limits
    }

    pub async fn validate_network_request(&self, url: &str) -> Result<()> {
        if !self.limits.allow_network {
            return Err(anyhow!("Network access is disabled in this sandbox"));
        }

        let parsed = url::Url::parse(url)
            .map_err(|e| anyhow!("Invalid URL: {}", e))?;

        if let Some(host) = parsed.host_str() {
            if !self.is_domain_allowed(host) {
                return Err(anyhow!(
                    "Domain '{}' is not in the allowed list. Allowed domains: {:?}",
                    host,
                    self.limits.allowed_domains
                ));
            }
        }

        Ok(())
    }
}

pub struct NetworkPolicy {
    allowed_schemes: HashSet<String>,
    blocked_ports: HashSet<u16>,
    max_request_size: usize,
    allowed_methods: HashSet<String>,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        let mut allowed_schemes = HashSet::new();
        allowed_schemes.insert("http".to_string());
        allowed_schemes.insert("https".to_string());

        let mut blocked_ports = HashSet::new();
        blocked_ports.insert(22);
        blocked_ports.insert(23);
        blocked_ports.insert(3306);
        blocked_ports.insert(5432);
        blocked_ports.insert(27017);
        blocked_ports.insert(6379);

        let mut allowed_methods = HashSet::new();
        allowed_methods.insert("GET".to_string());
        allowed_methods.insert("POST".to_string());
        allowed_methods.insert("HEAD".to_string());

        Self {
            allowed_schemes,
            blocked_ports,
            max_request_size: 10 * 1024 * 1024,
            allowed_methods,
        }
    }
}

impl NetworkPolicy {
    pub fn validate(&self, url: &str, method: &str) -> Result<()> {
        let parsed = url::Url::parse(url)
            .map_err(|e| anyhow!("Invalid URL: {}", e))?;

        if let Some(scheme) = parsed.scheme() {
            if !self.allowed_schemes.contains(scheme) {
                return Err(anyhow!("Scheme '{}' is not allowed", scheme));
            }
        }

        if let Some(port) = parsed.port() {
            if self.blocked_ports.contains(&port) {
                return Err(anyhow!("Port {} is blocked", port));
            }
        }

        if !self.allowed_methods.contains(method) {
            return Err(anyhow!("Method '{}' is not allowed", method));
        }

        Ok(())
    }

    pub fn max_request_size(&self) -> usize {
        self.max_request_size
    }
}
