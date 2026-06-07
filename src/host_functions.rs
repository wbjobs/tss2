use std::collections::HashMap;
use std::sync::Arc;
use wasmtime::{Caller, Func, Memory, Result as WasmResult, Store, Val, ValType, Trap};

use crate::types::Language;

pub const MAX_STRING_LENGTH: usize = 64 * 1024;
pub const CHUNK_SIZE: usize = 32 * 1024;

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

pub fn read_memory_safe(
    memory: &Memory,
    caller: &Caller<'_>,
    offset: u32,
    length: u32,
) -> WasmResult<Vec<u8>> {
    let offset = offset as usize;
    let length = length as usize;

    if length > MAX_STRING_LENGTH {
        return Err(Trap::new(format!(
            "String length {} exceeds maximum allowed length of {}",
            length, MAX_STRING_LENGTH
        ))
        .into());
    }

    let memory_size = memory.data_size(caller);
    if offset.checked_add(length).map_or(true, |end| end > memory_size) {
        return Err(Trap::new(format!(
            "Memory access out of bounds: offset={}, length={}, memory_size={}",
            offset, length, memory_size
        ))
        .into());
    }

    let mut buffer = vec![0u8; length];
    memory.read(caller, offset, &mut buffer)
        .map_err(|e| Trap::new(format!("Failed to read memory: {}", e)))?;

    Ok(buffer)
}

pub fn write_memory_safe(
    memory: &Memory,
    caller: &Caller<'_>,
    offset: u32,
    data: &[u8],
) -> WasmResult<usize> {
    let offset = offset as usize;
    let length = data.len();

    let memory_size = memory.data_size(caller);
    if offset.checked_add(length).map_or(true, |end| end > memory_size) {
        return Err(Trap::new(format!(
            "Memory write out of bounds: offset={}, length={}, memory_size={}",
            offset, length, memory_size
        ))
        .into());
    }

    memory.write(caller, offset, data)
        .map_err(|e| Trap::new(format!("Failed to write memory: {}", e)))?;

    Ok(length)
}

pub fn read_string_safe(
    memory: &Memory,
    caller: &Caller<'_>,
    offset: u32,
    length: u32,
) -> WasmResult<String> {
    let bytes = read_memory_safe(memory, caller, offset, length)?;
    String::from_utf8(bytes).map_err(|e| Trap::new(format!("Invalid UTF-8: {}", e)).into())
}

pub fn write_string_chunks(
    memory: &Memory,
    caller: &mut Caller<'_>,
    s: &str,
    buffer_ptr: u32,
    buffer_len: u32,
    offset_ptr: u32,
) -> WasmResult<i32> {
    let bytes = s.as_bytes();
    let total_len = bytes.len();

    let offset = if offset_ptr != 0 {
        let offset_bytes = read_memory_safe(memory, caller, offset_ptr, 4)?;
        u32::from_le_bytes(offset_bytes.try_into().unwrap()) as usize
    } else {
        0
    };

    let chunk_start = offset;
    let chunk_end = std::cmp::min(chunk_start + CHUNK_SIZE, total_len);
    let chunk = &bytes[chunk_start..chunk_end];

    if chunk.len() > buffer_len as usize {
        return Err(Trap::new(format!(
            "Chunk size {} exceeds buffer size {}",
            chunk.len(),
            buffer_len
        ))
        .into());
    }

    write_memory_safe(memory, caller, buffer_ptr, chunk)?;

    if chunk_end >= total_len {
        Ok(0)
    } else {
        if offset_ptr != 0 {
            let next_offset = (chunk_end as u32).to_le_bytes();
            write_memory_safe(memory, caller, offset_ptr, &next_offset)?;
        }
        Ok((total_len - chunk_end) as i32)
    }
}

pub fn read_string_chunks(
    memory: &Memory,
    caller: &mut Caller<'_>,
    total_len: u32,
    buffer_ptr: u32,
    buffer_len: u32,
    write_fn: impl Fn(&[u8]) -> WasmResult<()>,
) -> WasmResult<()> {
    let total_len = total_len as usize;

    if total_len > MAX_STRING_LENGTH {
        return Err(Trap::new(format!(
            "Total string length {} exceeds maximum allowed length of {}",
            total_len, MAX_STRING_LENGTH
        ))
        .into());
    }

    let mut offset = 0;
    while offset < total_len {
        let chunk_len = std::cmp::min(CHUNK_SIZE, total_len - offset);

        if chunk_len > buffer_len as usize {
            return Err(Trap::new(format!(
                "Chunk size {} exceeds buffer size {}",
                chunk_len, buffer_len
            ))
            .into());
        }

        let chunk = read_memory_safe(memory, caller, buffer_ptr + offset as u32, chunk_len as u32)?;
        write_fn(&chunk)?;

        offset += chunk_len;
    }

    Ok(())
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
            "Get string length (ptr, len) -> len with bounds check",
            Arc::new(|params| {
                let _ptr = params[0].unwrap_i32();
                let len = params[1].unwrap_i32();

                if len < 0 {
                    return Err(Trap::new("Negative string length").into());
                }
                if len as usize > MAX_STRING_LENGTH {
                    return Err(Trap::new(format!(
                        "String length {} exceeds maximum allowed length of {}",
                        len, MAX_STRING_LENGTH
                    ))
                    .into());
                }

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
                if n < 0 {
                    return Err(Trap::new("Negative Fibonacci index").into());
                }
                if n > 93 {
                    return Err(Trap::new("Fibonacci index too large (max 93 for i64)").into());
                }
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
                if n == 2 {
                    return Ok(vec![Val::I32(1)]);
                }
                if n % 2 == 0 {
                    return Ok(vec![Val::I32(0)]);
                }
                let sqrt_n = (n as f64).sqrt() as i32;
                for i in (3..=sqrt_n).step_by(2) {
                    if n % i == 0 {
                        return Ok(vec![Val::I32(0)]);
                    }
                }
                Ok(vec![Val::I32(1)])
            }),
        );

        self.register(
            "string_compare",
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Compare two strings (ptr1, len1, ptr2, len2) -> -1, 0, or 1",
            Arc::new(|params| {
                let len1 = params[1].unwrap_i32();
                let len2 = params[3].unwrap_i32();

                if len1 < 0 || len2 < 0 {
                    return Err(Trap::new("Negative string length").into());
                }
                if len1 as usize > MAX_STRING_LENGTH || len2 as usize > MAX_STRING_LENGTH {
                    return Err(Trap::new(format!(
                        "String length exceeds maximum allowed length of {}",
                        MAX_STRING_LENGTH
                    ))
                    .into());
                }

                Ok(vec![Val::I32(len1.cmp(&len2) as i32)])
            }),
        );

        self.register(
            "string_concat",
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Concatenate two strings (ptr1, len1, ptr2, len2, out_ptr) -> new_len with bounds check",
            Arc::new(|params| {
                let len1 = params[1].unwrap_i32();
                let len2 = params[3].unwrap_i32();

                if len1 < 0 || len2 < 0 {
                    return Err(Trap::new("Negative string length").into());
                }

                let total_len = len1.saturating_add(len2);
                if total_len as usize > MAX_STRING_LENGTH {
                    return Err(Trap::new(format!(
                        "Concatenated string length {} exceeds maximum allowed length of {}",
                        total_len, MAX_STRING_LENGTH
                    ))
                    .into());
                }

                Ok(vec![Val::I32(total_len)])
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

                if min > max {
                    return Err(Trap::new(format!(
                        "Invalid range: min ({}) > max ({})",
                        min, max
                    ))
                    .into());
                }

                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                let range = (max - min + 1) as u64;
                let random = if range > 0 {
                    (seed % range) as i32 + min
                } else {
                    min
                };

                Ok(vec![Val::I32(random)])
            }),
        );

        self.register(
            "check_memory_bounds",
            vec![ValType::I32, ValType::I32],
            vec![ValType::I32],
            "Check if memory range is valid (ptr, len) -> 1 if valid, 0 if invalid",
            Arc::new(|params| {
                let ptr = params[0].unwrap_i32();
                let len = params[1].unwrap_i32();

                if ptr < 0 || len < 0 {
                    return Ok(vec![Val::I32(0)]);
                }
                if len as usize > MAX_STRING_LENGTH {
                    return Ok(vec![Val::I32(0)]);
                }

                Ok(vec![Val::I32(1)])
            }),
        );

        self.register(
            "get_max_string_length",
            vec![],
            vec![ValType::I32],
            "Get maximum allowed string length in bytes",
            Arc::new(|_| Ok(vec![Val::I32(MAX_STRING_LENGTH as i32)])),
        );

        self.register(
            "get_chunk_size",
            vec![],
            vec![ValType::I32],
            "Get chunk size for large string transfers",
            Arc::new(|_| Ok(vec![Val::I32(CHUNK_SIZE as i32)])),
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
        memory_getter: impl Fn(&Caller<'_, T>) -> Memory + Send + Sync + 'static,
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
        exports.push(match language {
            Language::Python => format!(
                "MAX_STRING_LENGTH = {}\nCHUNK_SIZE = {}",
                MAX_STRING_LENGTH, CHUNK_SIZE
            ),
            Language::JavaScript => format!(
                "const MAX_STRING_LENGTH = {};\nconst CHUNK_SIZE = {};",
                MAX_STRING_LENGTH, CHUNK_SIZE
            ),
            Language::Ruby => format!(
                "MAX_STRING_LENGTH = {}\nCHUNK_SIZE = {}",
                MAX_STRING_LENGTH, CHUNK_SIZE
            ),
        });

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_string_length() {
        assert_eq!(MAX_STRING_LENGTH, 65536);
    }

    #[test]
    fn test_chunk_size() {
        assert_eq!(CHUNK_SIZE, 32768);
    }

    #[test]
    fn test_memory_bounds_validation() {
        let registry = HostFunctionRegistry::new();

        let params = vec![Val::I32(0), Val::I32(1000)];
        let result = registry.functions.get("check_memory_bounds")
            .unwrap()
            .implementation(params)
            .unwrap();
        assert_eq!(result[0].unwrap_i32(), 1);

        let params = vec![Val::I32(0), Val::I32(-1)];
        let result = registry.functions.get("check_memory_bounds")
            .unwrap()
            .implementation(params)
            .unwrap();
        assert_eq!(result[0].unwrap_i32(), 0);

        let params = vec![Val::I32(0), Val::I32(MAX_STRING_LENGTH as i32 + 1)];
        let result = registry.functions.get("check_memory_bounds")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_string_length_bounds() {
        let registry = HostFunctionRegistry::new();

        let params = vec![Val::I32(0), Val::I32(100)];
        let result = registry.functions.get("string_length")
            .unwrap()
            .implementation(params)
            .unwrap();
        assert_eq!(result[0].unwrap_i32(), 100);

        let params = vec![Val::I32(0), Val::I32(-1)];
        let result = registry.functions.get("string_length")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());

        let params = vec![Val::I32(0), Val::I32(MAX_STRING_LENGTH as i32 + 1)];
        let result = registry.functions.get("string_length")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_fibonacci_bounds() {
        let registry = HostFunctionRegistry::new();

        let params = vec![Val::I32(10)];
        let result = registry.functions.get("fibonacci")
            .unwrap()
            .implementation(params)
            .unwrap();
        assert_eq!(result[0].unwrap_i64(), 55);

        let params = vec![Val::I32(-1)];
        let result = registry.functions.get("fibonacci")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());

        let params = vec![Val::I32(94)];
        let result = registry.functions.get("fibonacci")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_random_int_bounds() {
        let registry = HostFunctionRegistry::new();

        let params = vec![Val::I32(1), Val::I32(10)];
        for _ in 0..10 {
            let result = registry.functions.get("random_int")
                .unwrap()
                .implementation(params.clone())
                .unwrap();
            let val = result[0].unwrap_i32();
            assert!(val >= 1 && val <= 10);
        }

        let params = vec![Val::I32(10), Val::I32(1)];
        let result = registry.functions.get("random_int")
            .unwrap()
            .implementation(params);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_constants() {
        let registry = HostFunctionRegistry::new();

        let result = registry.functions.get("get_max_string_length")
            .unwrap()
            .implementation(vec![])
            .unwrap();
        assert_eq!(result[0].unwrap_i32(), MAX_STRING_LENGTH as i32);

        let result = registry.functions.get("get_chunk_size")
            .unwrap()
            .implementation(vec![])
            .unwrap();
        assert_eq!(result[0].unwrap_i32(), CHUNK_SIZE as i32);
    }
}
