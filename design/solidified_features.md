# Currently Implementing

## Minimal Native Compiler
The compiler currently supports a single-source file
- Top-level immutable `comp` bindings
- Anonymous function expressions with no parameters
- built-in types `u8`, `i8`, `u16`, `i16`, `u32`, `i32`, `u64`, `i64`,
  `usize`, `isize`, `bool`, and `unit`
- contextually typed integer literals with optional integer type suffixes
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
type into signed and unsigned 8-, 16-, 32-, and 64-bit integers together with
target-sized `usize` and `isize`. An unsuffixed integer literal adopts an
expected integer type when one is available and otherwise defaults to `i64`.
Explicit suffixes such as `5i32`, `5u8`, and `5usize` select a type directly.

Unary negation is represented explicitly for general expressions. Directly
negated literals are elaborated as one mathematical value so signed minimum
values such as `-128i8` and `-9223372036854775808i64` are accepted without
first requiring their positive magnitudes to fit. Unsigned negation is
rejected. HIR and compile-time integer values use `i128`, allowing signed
compile-time subtraction and arithmetic results to be range-checked against
the selected language type.

LLVM lowering uses the corresponding integer width and selects signed or
unsigned comparisons and division from the semantic integer type. `usize` and
`isize` use LLVM target data for their pointer-sized representation. The
semantic target width is checked against LLVM's target width, and array lengths
and indices use `usize`.

This integer implementation remains incomplete:

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
