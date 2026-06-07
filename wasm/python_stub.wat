(module
  (memory (export "memory") 1)
  (global $__stack_pointer (mut i32) (i32.const 65536))

  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))

  (import "env" "add"
    (func $add (param i32 i32) (result i32)))
  (import "env" "multiply"
    (func $multiply (param i32 i32) (result i32)))
  (import "env" "fibonacci"
    (func $fibonacci (param i32) (result i64)))
  (import "env" "is_prime"
    (func $is_prime (param i32) (result i32)))
  (import "env" "get_timestamp"
    (func $get_timestamp (result i64)))
  (import "env" "random_int"
    (func $random_int (param i32 i32) (result i32)))
  (import "env" "string_length"
    (func $string_length (param i32 i32) (result i32)))
  (import "env" "string_compare"
    (func $string_compare (param i32 i32 i32 i32) (result i32)))
  (import "env" "string_concat"
    (func $string_concat (param i32 i32 i32 i32 i32) (result i32)))
  (import "env" "check_memory_bounds"
    (func $check_memory_bounds (param i32 i32) (result i32)))
  (import "env" "get_max_string_length"
    (func $get_max_string_length (result i32)))
  (import "env" "get_chunk_size"
    (func $get_chunk_size (result i32)))

  (func (export "_start")
    (nop))

  (func (export "main") (result i32)
    (i32.const 0))

  (func (export "call_host_add") (param i32 i32) (result i32)
    (call $add (local.get 0) (local.get 1)))

  (func (export "call_host_fibonacci") (param i32) (result i64)
    (call $fibonacci (local.get 0)))

  (func (export "call_host_string_length") (param i32 i32) (result i32)
    (call $string_length (local.get 0) (local.get 1)))

  (func (export "call_host_check_bounds") (param i32 i32) (result i32)
    (call $check_memory_bounds (local.get 0) (local.get 1)))
)
