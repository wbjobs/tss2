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

  (func (export "_start")
    (nop))

  (func (export "main") (result i32)
    (i32.const 0))
)
