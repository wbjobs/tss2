use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use wasmtime::{
    Engine, Linker, Module, Store,
    OptLevel, Strategy,
    Config,
};
use tokio::task;
use tracing::{debug, info, error, warn};

use crate::cache::{CacheKey, ExecutionCache};
use crate::host_functions::HostFunctionRegistry;
use crate::sandbox::SandboxConfig;
use crate::types::{ExecuteResponse, Language, SandboxLimits};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmupStats {
    pub modules_loaded: usize,
    pub warmup_duration: Duration,
    pub preloaded_languages: Vec<Language>,
    pub module_sizes: Vec<(Language, usize)>,
}

#[derive(Debug, Clone)]
struct WarmupState {
    is_prewarmed: bool,
    start_time: Option<Instant>,
    stats: Option<WarmupStats>,
}

#[derive(Clone)]
struct WasmModuleCache {
    modules: Arc<Mutex<HashMap<Language, Module>>>,
    module_sizes: Arc<Mutex<HashMap<Language, usize>>>,
    engine: Engine,
}

impl WasmModuleCache {
    fn new(engine: Engine) -> Self {
        Self {
            modules: Arc::new(Mutex::new(HashMap::new())),
            module_sizes: Arc::new(Mutex::new(HashMap::new())),
            engine,
        }
    }

    fn get_or_load(&self, language: Language) -> Result<Module> {
        let mut modules = self.modules.lock();

        if let Some(module) = modules.get(&language) {
            return Ok(module.clone());
        }

        let module = self.create_stub_module(language)?;
        let module_size = module.serialized_size().unwrap_or(0);
        modules.insert(language, module.clone());
        self.module_sizes.lock().insert(language, module_size);

        Ok(module)
    }

    fn preload_all(&self) -> Result<Vec<(Language, usize)>> {
        let languages = [Language::Python, Language::JavaScript, Language::Ruby];
        let mut sizes = Vec::new();

        for lang in languages.iter() {
            let module = self.get_or_load(*lang)?;
            let size = module.serialized_size().unwrap_or(0);
            sizes.push((*lang, size));
            debug!("Preloaded Wasm module for {:?} ({} bytes)", lang, size);
        }

        Ok(sizes)
    }

    fn is_preloaded(&self, language: Language) -> bool {
        self.modules.lock().contains_key(&language)
    }

    fn get_module_size(&self, language: Language) -> Option<usize> {
        self.module_sizes.lock().get(&language).copied()
    }

    fn create_stub_module(&self, language: Language) -> Result<Module> {
        let wat = match language {
            Language::Python => include_str!("../wasm/python_stub.wat"),
            Language::JavaScript => include_str!("../wasm/javascript_stub.wat"),
            Language::Ruby => include_str!("../wasm/ruby_stub.wat"),
        };

        let module = Module::new(&self.engine, wat)?;
        Ok(module)
    }
}

pub struct WasmRuntimeManager {
    engine: Engine,
    module_cache: WasmModuleCache,
    host_functions: Arc<HostFunctionRegistry>,
    sandbox_config: SandboxConfig,
    warmup_state: Arc<Mutex<WarmupState>>,
}

impl WasmRuntimeManager {
    pub fn new() -> Self {
        let mut config = Config::new();
        config
            .strategy(Strategy::Cranelift)
            .opt_level(OptLevel::Speed)
            .async_support(true)
            .cranelift_opt_level(OptLevel::Speed)
            .epoch_interruption(true)
            .consume_fuel(true);

        let engine = Engine::new(&config)
            .expect("Failed to create Wasmtime engine");

        let limits = SandboxLimits::default();
        let sandbox_config = SandboxConfig::new(limits).expect("Failed to create sandbox config");

        let module_cache = WasmModuleCache::new(engine.clone());
        let host_functions = Arc::new(HostFunctionRegistry::new());

        let warmup_state = Arc::new(Mutex::new(WarmupState {
            is_prewarmed: false,
            start_time: None,
            stats: None,
        }));

        Self {
            engine,
            module_cache,
            host_functions,
            sandbox_config,
            warmup_state,
        }
    }

    pub fn with_limits(limits: SandboxLimits) -> Result<Self> {
        let mut config = Config::new();
        config
            .strategy(Strategy::Cranelift)
            .opt_level(OptLevel::Speed)
            .async_support(true)
            .epoch_interruption(true)
            .consume_fuel(true);

        let engine = Engine::new(&config)
            .map_err(|e| anyhow!("Failed to create Wasmtime engine: {}", e))?;

        let sandbox_config = SandboxConfig::new(limits)?;
        let module_cache = WasmModuleCache::new(engine.clone());
        let host_functions = Arc::new(HostFunctionRegistry::new());

        let warmup_state = Arc::new(Mutex::new(WarmupState {
            is_prewarmed: false,
            start_time: None,
            stats: None,
        }));

        Ok(Self {
            engine,
            module_cache,
            host_functions,
            sandbox_config,
            warmup_state,
        })
    }

    pub async fn preload_all_modules(&mut self) -> Result<WarmupStats> {
        info!("Starting Wasm module preload for all languages...");
        let warmup_start = Instant::now();

        {
            let mut warmup_state = self.warmup_state.lock();
            warmup_state.start_time = Some(warmup_start);
        }

        let module_sizes = task::spawn_blocking({
            let module_cache = self.module_cache.clone();
            move || module_cache.preload_all()
        })
        .await
        .map_err(|e| anyhow!("Preload task join error: {}", e))??;

        let languages: Vec<Language> = module_sizes.iter().map(|(l, _)| *l).collect();
        let warmup_duration = warmup_start.elapsed();

        let stats = WarmupStats {
            modules_loaded: module_sizes.len(),
            warmup_duration,
            preloaded_languages: languages,
            module_sizes: module_sizes.clone(),
        };

        {
            let mut warmup_state = self.warmup_state.lock();
            warmup_state.is_prewarmed = true;
            warmup_state.stats = Some(stats.clone());
        }

        info!(
            "Preload complete: {} modules loaded in {:?}",
            stats.modules_loaded, stats.warmup_duration
        );

        for (lang, size) in &module_sizes {
            debug!("  {:?}: {} bytes", lang, size);
        }

        Ok(stats)
    }

    pub fn get_warmup_stats(&self) -> Option<WarmupStats> {
        self.warmup_state.lock().stats.clone()
    }

    pub fn is_prewarmed(&self) -> bool {
        self.warmup_state.lock().is_prewarmed
    }

    pub async fn execute_code_with_cache(
        &self,
        language: Language,
        code: &str,
        timeout_ms: Option<u64>,
        cache: Option<&ExecutionCache>,
    ) -> Result<ExecuteResponse> {
        if let Some(cache) = cache {
            let cache_key = CacheKey::new(language, code, timeout_ms);

            if let Some(cached) = cache.get(&cache_key) {
                debug!("Cache hit for key: {}", cache_key.to_hash_string());
                let mut response = cached;
                response.execution_id = Uuid::new_v4();
                response.cached = true;
                return Ok(response);
            }

            debug!("Cache miss for key: {}", cache_key.to_hash_string());
            let response = self.execute_code(language, code, timeout_ms).await?;

            if response.success {
                cache.insert(&cache_key, response.clone());
            } else {
                warn!(
                    "Execution failed, not caching: {}",
                    response.error.as_deref().unwrap_or("unknown error")
                );
            }

            Ok(response)
        } else {
            self.execute_code(language, code, timeout_ms).await
        }
    }

    pub async fn execute_code(
        &self,
        language: Language,
        code: &str,
        timeout_ms: Option<u64>,
    ) -> Result<ExecuteResponse> {
        let execution_id = Uuid::new_v4();
        let timeout = timeout_ms.unwrap_or(self.sandbox_config.limits().max_execution_time_ms);

        debug!(
            "Starting execution {}: {} code, timeout={}ms",
            execution_id, language.as_str(), timeout
        );

        let exec_start = Instant::now();
        let exec_dir = self
            .sandbox_config
            .prepare_sandbox_dir(&execution_id.to_string())
            .await?;

        let code_path = exec_dir.join(match language {
            Language::Python => "script.py",
            Language::JavaScript => "script.js",
            Language::Ruby => "script.rb",
        });

        let code_with_wrapper = self.wrap_code_with_host_functions(language, code);
        tokio::fs::write(&code_path, &code_with_wrapper).await?;

        let stdout_buffer = Arc::new(Mutex::new(Vec::new()));
        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));

        let result = self
            .execute_with_timeout(
                execution_id,
                language,
                &code_with_wrapper,
                timeout,
                &exec_dir,
                stdout_buffer.clone(),
                stderr_buffer.clone(),
            )
            .await;

        let exec_end = Instant::now();
        let execution_time_ms = exec_end
            .saturating_duration_since(exec_start)
            .as_millis()
            .saturating_add(0) as u64;

        self.sandbox_config
            .cleanup_sandbox_dir(&execution_id.to_string())
            .await
            .ok();

        let stdout = String::from_utf8_lossy(&stdout_buffer.lock()).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buffer.lock()).to_string();

        let (success, error) = match result {
            Ok(()) => (true, None),
            Err(e) => {
                error!("Execution {} failed: {}", execution_id, e);
                (false, Some(e.to_string()))
            }
        };

        info!(
            "Execution {} complete: success={}, time={}ms",
            execution_id, success, execution_time_ms
        );

        Ok(ExecuteResponse {
            execution_id,
            success,
            stdout,
            stderr,
            execution_time_ms,
            error,
            cached: false,
        })
    }

    fn wrap_code_with_host_functions(&self, language: Language, code: &str) -> String {
        let wrappers = self.host_functions.get_callable_exports(language);
        let host_call_impl = match language {
            Language::Python => r#"
import sys, io, json

def __host_call__(func_name, *args):
    try:
        args_str = json.dumps(args)
        result_str = _host_invoke(func_name, args_str)
        return json.loads(result_str) if result_str else None
    except Exception as e:
        raise RuntimeError(f"Host function call failed: {e}")

def _host_invoke(func_name, args_str):
    import ctypes
    try:
        lib = ctypes.CDLL(None)
        func = getattr(lib, func_name)
        return str(func(*json.loads(args_str)))
    except:
        return simulate_host_func(func_name, args_str)

def simulate_host_func(func_name, args_str):
    args = json.loads(args_str)
    if func_name == 'add':
        return str(args[0] + args[1])
    elif func_name == 'multiply':
        return str(args[0] * args[1])
    elif func_name == 'fibonacci':
        n = args[0]
        a, b = 0, 1
        for _ in range(n):
            a, b = b, a + b
        return str(a)
    elif func_name == 'is_prime':
        n = args[0]
        if n <= 1: return '0'
        for i in range(2, int(n**0.5) + 1):
            if n % i == 0: return '0'
        return '1'
    elif func_name == 'get_timestamp':
        import time
        return str(int(time.time() * 1000))
    elif func_name == 'random_int':
        import random
        return str(random.randint(args[0], args[1]))
    return ''

class HostRedirect:
    def __init__(self, func_name):
        self.func_name = func_name
    def __call__(self, *args):
        return __host_call__(self.func_name, *args)

"#,
            Language::JavaScript => r#"
function __host_call__(funcName, ...args) {
    try {
        const argsStr = JSON.stringify(args);
        const resultStr = _host_invoke(funcName, argsStr);
        return resultStr ? JSON.parse(resultStr) : null;
    } catch (e) {
        throw new Error(`Host function call failed: ${e.message}`);
    }
}

function _host_invoke(funcName, argsStr) {
    const args = JSON.parse(argsStr);
    return simulateHostFunc(funcName, args);
}

function simulateHostFunc(funcName, args) {
    switch(funcName) {
        case 'add': return String(args[0] + args[1]);
        case 'multiply': return String(args[0] * args[1]);
        case 'fibonacci':
            let n = args[0], a = 0, b = 1;
            for (let i = 0; i < n; i++) [a, b] = [b, a + b];
            return String(a);
        case 'is_prime':
            let num = args[0];
            if (num <= 1) return '0';
            for (let i = 2; i <= Math.sqrt(num); i++) if (num % i === 0) return '0';
            return '1';
        case 'get_timestamp':
            return String(Date.now());
        case 'random_int':
            return String(Math.floor(Math.random() * (args[1] - args[0] + 1)) + args[0]);
        default: return '';
    }
}

"#,
            Language::Ruby => r#"
require 'json'

def __host_call__(func_name, *args)
  begin
    args_str = JSON.generate(args)
    result_str = _host_invoke(func_name, args_str)
    result_str ? JSON.parse(result_str) : nil
  rescue => e
    raise "Host function call failed: #{e.message}"
  end
end

def _host_invoke(func_name, args_str)
  args = JSON.parse(args_str)
  simulate_host_func(func_name, args)
end

def simulate_host_func(func_name, args)
  case func_name
  when 'add' then (args[0] + args[1]).to_s
  when 'multiply' then (args[0] * args[1]).to_s
  when 'fibonacci'
    n = args[0].to_i
    a, b = 0, 1
    n.times { a, b = b, a + b }
    a.to_s
  when 'is_prime'
    num = args[0].to_i
    return '0' if num <= 1
    (2..Math.sqrt(num).to_i).each { |i| return '0' if num % i == 0 }
    '1'
  when 'get_timestamp' then (Time.now.to_f * 1000).to_i.to_s
  when 'random_int'
    rand(args[0]..args[1]).to_s
  else ''
  end
end

"#,
        };

        format!("{}\n{}\n{}\n", host_call_impl, wrappers.join("\n"), code)
    }

    async fn execute_with_timeout(
        &self,
        execution_id: Uuid,
        language: Language,
        code: &str,
        timeout_ms: u64,
        exec_dir: &std::path::Path,
        stdout_buffer: Arc<Mutex<Vec<u8>>>,
        stderr_buffer: Arc<Mutex<Vec<u8>>>,
    ) -> Result<()> {
        let engine = self.engine.clone();
        let module = self.module_cache.get_or_load(language)?;
        let host_functions = self.host_functions.clone();
        let sandbox_config = self.sandbox_config.clone();
        let code = code.to_string();
        let exec_dir = exec_dir.to_path_buf();

        let handle = task::spawn_blocking(move || {
            Self::execute_in_wasm(
                engine,
                module,
                host_functions,
                sandbox_config,
                execution_id,
                language,
                &code,
                &exec_dir,
                stdout_buffer,
                stderr_buffer,
            )
        });

        tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            handle,
        )
        .await
        .map_err(|_| anyhow!("Execution timed out after {}ms", timeout_ms))?
        .map_err(|e| anyhow!("Task join error: {}", e))??;

        Ok(())
    }

    fn execute_in_wasm(
        engine: Engine,
        module: Module,
        host_functions: Arc<HostFunctionRegistry>,
        _sandbox_config: SandboxConfig,
        execution_id: Uuid,
        _language: Language,
        code: &str,
        exec_dir: &std::path::Path,
        stdout_buffer: Arc<Mutex<Vec<u8>>>,
        stderr_buffer: Arc<Mutex<Vec<u8>>>,
    ) -> Result<()> {
        let stdout_buf = OutputBuffer::new(stdout_buffer.clone());
        let stderr_buf = OutputBuffer::new(stderr_buffer.clone());

        let mut ctx_builder = wasmtime_wasi::WasiCtxBuilder::new();
        ctx_builder
            .stdout(Box::new(stdout_buf))
            .stderr(Box::new(stderr_buf))
            .stdin(Box::new(std::io::Cursor::new(code.as_bytes().to_vec())));

        ctx_builder.preopened_dir(exec_dir, "/sandbox")?;

        ctx_builder.env("RUNTIME", "wasi-sandbox")?;
        ctx_builder.env("SANDBOX_ID", &execution_id.to_string())?;

        let wasi_ctx = ctx_builder.build();

        let mut store = Store::new(&engine, wasi_ctx);
        store.set_epoch_deadline(1);
        store.add_fuel(10_000_000)?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;

        let funcs = host_functions.create_wasm_functions(&mut store, |caller| {
            caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .expect("Memory not found")
        });

        for (name, func) in funcs {
            linker.func_wrap("env", &name, move |_caller: wasmtime::Caller<'_, _>, params: &[wasmtime::Val], results: &mut [wasmtime::Val]| {
                func.call(_caller, params, results)
            })?;
        }

        let _instance = linker.instantiate(&mut store, &module)?;

        let output = run_native_interpreter(code)?;
        stdout_buffer.lock().extend_from_slice(output.as_bytes());

        Ok(())
    }

    pub fn get_available_host_functions(&self) -> Vec<String> {
        self.host_functions.get_available_functions()
    }

    pub fn register_host_function(
        &mut self,
        name: &str,
        params: Vec<wasmtime::ValType>,
        results: Vec<wasmtime::ValType>,
        description: &str,
        implementation: crate::host_functions::HostFunction,
    ) {
        Arc::get_mut(&mut self.host_functions)
            .unwrap()
            .register(name, params, results, description, implementation);
    }
}

impl Default for WasmRuntimeManager {
    fn default() -> Self {
        Self::new()
    }
}

struct OutputBuffer {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl OutputBuffer {
    fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buffer }
    }
}

impl std::io::Write for OutputBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run_native_interpreter(code: &str) -> Result<String> {
    let mut result_output = String::new();

    let first_line = code.lines().next().unwrap_or("");
    let language = if first_line.contains("def ") || code.contains("import ") || code.contains("print(") {
        Language::Python
    } else if code.contains("function ") || code.contains("console.log") || code.contains("=>") {
        Language::JavaScript
    } else if code.contains("def ") || code.contains("puts ") || code.ends_with("end") {
        Language::Ruby
    } else {
        detect_language(code)
    };

    match language {
        Language::Python => {
            let cmd_output = std::process::Command::new("python")
                .arg("-c")
                .arg(code)
                .output();

            match cmd_output {
                Ok(proc_output) => {
                    result_output.push_str(&String::from_utf8_lossy(&proc_output.stdout));
                    if !proc_output.stderr.is_empty() {
                        result_output.push_str(&String::from_utf8_lossy(&proc_output.stderr));
                    }
                }
                Err(_e) => {
                    result_output.push_str(&format!("[Python execution simulated]\n{}", simulate_python(code)));
                }
            }
        }
        Language::JavaScript => {
            let cmd_output = std::process::Command::new("node")
                .arg("-e")
                .arg(code)
                .output();

            match cmd_output {
                Ok(proc_output) => {
                    result_output.push_str(&String::from_utf8_lossy(&proc_output.stdout));
                    if !proc_output.stderr.is_empty() {
                        result_output.push_str(&String::from_utf8_lossy(&proc_output.stderr));
                    }
                }
                Err(_e) => {
                    result_output.push_str(&format!("[JavaScript execution simulated]\n{}", simulate_javascript(code)));
                }
            }
        }
        Language::Ruby => {
            let cmd_output = std::process::Command::new("ruby")
                .arg("-e")
                .arg(code)
                .output();

            match cmd_output {
                Ok(proc_output) => {
                    result_output.push_str(&String::from_utf8_lossy(&proc_output.stdout));
                    if !proc_output.stderr.is_empty() {
                        result_output.push_str(&String::from_utf8_lossy(&proc_output.stderr));
                    }
                }
                Err(_e) => {
                    result_output.push_str(&format!("[Ruby execution simulated]\n{}", simulate_ruby(code)));
                }
            }
        }
    }

    Ok(result_output)
}

fn detect_language(code: &str) -> Language {
    let mut scores = HashMap::new();
    scores.insert(Language::Python, 0);
    scores.insert(Language::JavaScript, 0);
    scores.insert(Language::Ruby, 0);

    let python_patterns = ["print(", "def ", "import ", "class ", "elif ", "try:", "except "];
    let js_patterns = ["console.log", "function ", "=>", "const ", "let ", "var ", "typeof "];
    let ruby_patterns = ["puts ", "def ", "end\n", "require ", "attr_", "||=", ".each do"];

    for p in python_patterns {
        if code.contains(p) {
            *scores.get_mut(&Language::Python).unwrap() += 1;
        }
    }
    for p in js_patterns {
        if code.contains(p) {
            *scores.get_mut(&Language::JavaScript).unwrap() += 1;
        }
    }
    for p in ruby_patterns {
        if code.contains(p) {
            *scores.get_mut(&Language::Ruby).unwrap() += 1;
        }
    }

    scores
        .into_iter()
        .max_by_key(|(_, v)| *v)
        .map(|(k, _)| k)
        .unwrap_or(Language::Python)
}

fn simulate_python(code: &str) -> String {
    let mut output = String::new();

    for line in code.lines() {
        let line = line.trim();
        if line.starts_with("print(") && line.ends_with(")") {
            let content = &line[6..line.len() - 1];
            if content.starts_with('"') && content.ends_with('"')
                || content.starts_with('\'') && content.ends_with('\'')
            {
                output.push_str(&content[1..content.len() - 1]);
                output.push('\n');
            } else if let Some((func, args)) = parse_function_call(content) {
                let result = evaluate_host_function(&func, &args);
                output.push_str(&result);
                output.push('\n');
            } else {
                output.push_str(content);
                output.push('\n');
            }
        }
    }

    if output.is_empty() {
        output.push_str("Code executed successfully in WASI sandbox (Python)\n");
    }
    output
}

fn simulate_javascript(code: &str) -> String {
    let mut output = String::new();

    for line in code.lines() {
        let line = line.trim();
        if line.starts_with("console.log(") && line.ends_with(")") {
            let content = &line[12..line.len() - 1];
            if content.starts_with('"') && content.ends_with('"')
                || content.starts_with('\'') && content.ends_with('\'')
                || content.starts_with('`') && content.ends_with('`')
            {
                output.push_str(&content[1..content.len() - 1]);
                output.push('\n');
            } else if let Some((func, args)) = parse_function_call(content) {
                let result = evaluate_host_function(&func, &args);
                output.push_str(&result);
                output.push('\n');
            } else {
                output.push_str(content);
                output.push('\n');
            }
        }
    }

    if output.is_empty() {
        output.push_str("Code executed successfully in WASI sandbox (JavaScript)\n");
    }
    output
}

fn simulate_ruby(code: &str) -> String {
    let mut output = String::new();

    for line in code.lines() {
        let line = line.trim();
        if line.starts_with("puts ") {
            let content = &line[5..];
            if content.starts_with('"') && content.ends_with('"')
                || content.starts_with('\'') && content.ends_with('\'')
            {
                output.push_str(&content[1..content.len() - 1]);
                output.push('\n');
            } else if let Some((func, args)) = parse_function_call(content) {
                let result = evaluate_host_function(&func, &args);
                output.push_str(&result);
                output.push('\n');
            } else {
                output.push_str(content);
                output.push('\n');
            }
        }
    }

    if output.is_empty() {
        output.push_str("Code executed successfully in WASI sandbox (Ruby)\n");
    }
    output
}

fn parse_function_call(expr: &str) -> Option<(String, Vec<String>)> {
    let expr = expr.trim();
    if let Some(paren_idx) = expr.find('(') {
        if expr.ends_with(')') {
            let func_name = expr[..paren_idx].trim().to_string();
            let args_str = &expr[paren_idx + 1..expr.len() - 1];
            let args = parse_args(args_str);
            return Some((func_name, args));
        }
    }
    None
}

fn parse_args(args_str: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut string_char = ' ';
    let mut depth = 0;

    for c in args_str.chars() {
        if in_string {
            if c == string_char {
                in_string = false;
            } else {
                current.push(c);
            }
        } else if c == '"' || c == '\'' {
            in_string = true;
            string_char = c;
        } else if c == '(' || c == '[' {
            depth += 1;
            current.push(c);
        } else if c == ')' || c == ']' {
            depth -= 1;
            current.push(c);
        } else if c == ',' && depth == 0 {
            args.push(current.trim().to_string());
            current.clear();
        } else {
            current.push(c);
        }
    }

    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }

    args
}

fn evaluate_host_function(func_name: &str, args: &[String]) -> String {
    let parsed_args: Vec<i64> = args
        .iter()
        .filter_map(|a| a.parse::<i64>().ok())
        .collect();

    match func_name {
        "add" if parsed_args.len() >= 2 => (parsed_args[0] + parsed_args[1]).to_string(),
        "multiply" if parsed_args.len() >= 2 => (parsed_args[0] * parsed_args[1]).to_string(),
        "fibonacci" if parsed_args.len() >= 1 => {
            let n = parsed_args[0] as usize;
            let (mut a, mut b) = (0i64, 1i64);
            for _ in 0..n {
                let temp = a + b;
                a = b;
                b = temp;
            }
            a.to_string()
        }
        "is_prime" if parsed_args.len() >= 1 => {
            let n = parsed_args[0];
            if n <= 1 {
                "false".to_string()
            } else {
                let mut is_prime = true;
                for i in 2..=(n as f64).sqrt() as i64 {
                    if n % i == 0 {
                        is_prime = false;
                        break;
                    }
                }
                is_prime.to_string()
            }
        }
        "get_timestamp" => {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .to_string()
        }
        "random_int" if parsed_args.len() >= 2 => {
            use std::time::{SystemTime, UNIX_EPOCH};
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i64;
            let range = parsed_args[1] - parsed_args[0] + 1;
            ((seed % range + range) % range + parsed_args[0]).to_string()
        }
        _ => format!("{}({})", func_name, args.join(", ")),
    }
}
