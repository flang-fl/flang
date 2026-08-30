# Currently Implementing

## Minimal Native Compiler
The compiler currently supports a single-source file
- Top-level immutable `comp` bindings
- Anonymous function expressions with no parameters
- built-in types `i32`, `i64`, `u32`, `usize`, `bool`, and `unit`
- contextually typed positive integer literals with optional `i32`, `i64`,
  `u32`, and `usize` suffixes
- `return` statements
- static checking of returned integer values against `i64`
- compile-time creation of function values
- function identity represented independently of the binding through `FunctionId`
- LLVM IR generation for a zero-parameter function returning one `i64` literal
- native executable generation through Clang

Current accepted example:
```text
comp main = fn() -> i64 {
    return 42;
}
```

## Generalized integer types

The current implementation generalizes the previous hard-coded `i64` value
type into signed `i32` and `i64`, unsigned `u32`, and `usize`. An unsuffixed
positive integer literal adopts an expected integer type when one is available
and otherwise defaults to `i64`. Explicit suffixes such as `5i32`, `5u32`, and
`5usize` select a type directly. Literal and compile-time arithmetic results are
checked against the positive range of the selected type. LLVM lowering uses
the corresponding integer width and selects signed or unsigned comparisons and
division from the semantic integer type.

This integer implementation remains incomplete:

- Negative literals and unary negation are not represented. Compile-time
  integer values currently use `u64`, so signed subtraction cannot produce a
  negative result and signed minimum values cannot be expressed.
- `usize` is temporarily lowered as LLVM `i64` rather than using target data,
  and array indices and lengths still expect `i64`.
- Only `i32`, `i64`, `u32`, and `usize` exist; the other fixed-width integer
  types have not been added.
- Numeric separators such as `4_000_000_000` are not tokenized.
- There are no explicit integer conversions, and mixed-width arithmetic is
  rejected rather than coerced.
- Literal inference is locally directional. For example, a suffix on the
  right-hand side of a comparison cannot yet revise an unsuffixed left operand
  after it has defaulted to `i64`.
- Compile-time arithmetic reports overflow, while the intended runtime
  overflow, division-by-zero, and signed-division behavior is not yet fully
  specified.
- Floating-point types and literals are not part of this implementation.

# Implemented

## Binary Operators
`+`, `-`, `*`, `/`, `==`, `!=`, `<=`, `<`, `>`, `>=`

## Comptime and Runtime function calls

## Comptime Function Binding
An anonymous function expression creates a function value.
A `comp` binding may bind that value to a name at compile time.

Bindings and function entities have distinct identities.
Multiple bindings may eventually refer to the same function entity.

## Minimal Phase Rule
A comptime expression may use values already available at comptime.
A runtime binding is unavailable to comptime evaluation.
