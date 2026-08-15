# Currently Implementing

## Minimal Native Compiler
The compiler currently supports a single-source file
- Top-level immutable `comp` bindings
- Anonymous function expressions with no parameters
- built-in types `i64` and `unit`
- integer literals
- `return` statements
- static checking of returned integer values against `i64`
- compile-time creation of function values
- function identity represented independently from the binding through `FunctionId`
- LLVM IR generation for a zero-parameter function returning one `i64` literal
- native executable generation through Clang

Current accepted example:
```text
comp main = fn() -> i64 {
    return 42;
}
```

# Implemented

## Comptime Function Binding
An anonymous function expression creates a function value.
A `comp` binding may bind that value to a name at compile time.

Bindings and function entities have distinct identities.
Multiple bindings may eventually refer to the same function entity.

## Minimal Phase Rule
A comptime expression may use values already available at comptime.
A runtime binding is unavailable to comptime evaluation.

