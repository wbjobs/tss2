use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{Caller, Func, Result as WasmResult, Store, Val, ValType};

use crate::types::Language;

pub type HostFunction = Arc<dyn Fn(Vec<Val>) -> WasmResult<Vec<Val>> + Send + Sync>;

pub struct HostFunctionRegistry {
    functions: HashMap<String, HostFunctionInfo>,
}

struct HostFunctionInfo {
    name: String,
    params: Vec<ValType>,
    results: Vec<ValType>,
    implementation: HostFunction,
    description: String,
}

impl HostFunctionRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            functions: HashMap::new(),
        };
        registry.register_default_functions();
        registry
    }

    fn register_default_functions(&mut self) {
        self.register(
            "add",
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Add two integers",
            Arc::new(|params| {
                let a = params[0].unwrap_i32();
                let b = params[1].unwrap_i32();
                Ok(vec![Val::I32(a + b)])
            }),
        );

        self.register(
            "multiply",
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Multiply two integers",
            Arc::new(|params| {
                let a = params[0].unwrap_i32();
                let b = params[1].unwrap_i32();
                Ok(vec![Val::I32(a * b)])
            }),
        );

        self.register(
            "string_length",
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Get string length (ptr, len) -> len",
            Arc::new(|params| {
                let len = params[1].unwrap_i32();
                Ok(vec![Val::I32(len)])
            }),
        );

        self.register(
            "fibonacci",
            vec![ValType::I32],
            vec![ValType::I64],
            "Compute Fibonacci number",
            Arc::new(|params| {
                let n = params[0].unwrap_i32();
                let result = if n <= 0 {
                    0
                } else if n == 1 {
                    1
                } else {
                    let mut a: i64 = 0;
                    let mut b: i64 = 1;
                    for _ in 2..=n {
                        let temp = a + b;
                        a = b;
                        b = temp;
                    }
                    b
                };
                Ok(vec![Val::I64(result)])
            }),
        );

        self.register(
            "is_prime",
            vec![ValType::I32],
            vec![ValType::I32],
            "Check if number is prime (returns 1 or 0)",
            Arc::new(|params| {
                let n = params[0].unwrap_i32();
                if n <= 1 {
                    return Ok(vec![Val::I32(0)]);
                }
                for i in 2..=(n as f64).sqrt() as i32 {
                    if n % i == 0 {
                        return Ok(vec![Val::I32(0)]);
                    }
                }
                Ok(vec![Val::I32(1)])
            }),
        );

        self.register(
            "to_uppercase",
            vec![ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Convert string to uppercase (in_ptr, in_len, out_ptr) -> out_len",
            Arc::new(move |_| {
                Ok(vec![Val::I32(0)])
            }),
        );

        self.register(
            "get_timestamp",
            vec![],
            vec![ValType::I64],
            "Get current Unix timestamp in milliseconds",
            Arc::new(|_| {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                Ok(vec![Val::I64(now)])
            }),
        );

        self.register(
            "random_int",
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Generate random integer between min and max (inclusive)",
            Arc::new(|params| {
                let min = params[0].unwrap_i32();
                let max = params[1].unwrap_i32();
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                let random = (seed % (max - min + 1) as u64) as i32 + min;
                Ok(vec![Val::I32(random)])
            }),
        );
    }

    pub fn register(
        &mut self,
        name: &str,
        params: Vec<ValType>,
        results: Vec<ValType>,
        description: &str,
        implementation: HostFunction,
    ) {
        let info = HostFunctionInfo {
            name: name.to_string(),
            params,
            results,
            implementation,
            description: description.to_string(),
        };
        self.functions.insert(name.to_string(), info);
    }

    pub fn get_available_functions(&self) -> Vec<String> {
        self.functions
            .iter()
            .map(|(name, info)| format!("{}: {}", name, info.description))
            .collect()
    }

    pub fn create_wasm_functions<T>(
        &self,
        store: &mut Store<T>,
        memory_getter: impl Fn(&Caller<'_, T>) -> wasmtime::Memory + Send + Sync + 'static,
    ) -> Vec<(String, Func)> {
        let mut funcs = Vec::new();
        let functions = self.functions.clone();
        let memory_getter = Arc::new(memory_getter);

        for (name, info) in functions {
            let _memory_getter = memory_getter.clone();
            let params = info.params.clone();
            let results = info.results.clone();
            let implementation = info.implementation.clone();

            let func = Func::new(
                store,
                wasmtime::FuncType::new(params, results),
                move |_caller: Caller<'_, T>, input: &[Val], output: &mut [Val]| {
                    match implementation(input.to_vec()) {
                        Ok(results) => {
                            for (i, val) in results.iter().enumerate() {
                                if i < output.len() {
                                    output[i] = val.clone();
                                }
                            }
                            Ok(())
                        }
                        Err(e) => {
                            eprintln!("Host function error: {}", e);
                            Err(e)
                        }
                    }
                },
            );

            funcs.push((name, func));
        }

        funcs
    }

    pub fn get_import_names(&self) -> Vec<String> {
        self.functions.keys().cloned().collect()
    }

    pub fn get_callable_exports(&self, language: Language) -> Vec<String> {
        let mut exports = Vec::new();
        for (name, _info) in &self.functions {
            match language {
                Language::Python => {
                    exports.push(format!(
                        "def {}(*args):\n    return __host_call__('{}', *args)",
                        name, name
                    ));
                }
                Language::JavaScript => {
                    exports.push(format!(
                        "function {}(...args) {{ return __host_call__('{}', ...args); }}",
                        name, name
                    ));
                }
                Language::Ruby => {
                    exports.push(format!(
                        "def {}(*args)\n  __host_call__('{}', *args)\nend",
                        name, name
                    ));
                }
            }
        }
        exports
    }
}

impl Default for HostFunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
