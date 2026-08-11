# Programming Language Design Seed

This document is the current design seed for an experimental statically typed programming language centered around unified compile-time/runtime execution, first-class program manipulation, and strong reflection.

It is intentionally not a finished specification. Treat firm decisions as defaults, and treat open questions as design space to explore carefully rather than silently resolving them.

---

## 1. Core goals

The language should be:

- Statically typed.
- Designed around compile-time execution as a first-class part of ordinary programming.
- Able to execute essentially any ordinary language code at compile time when its dependencies are available there.
- Able to inspect, create, and modify the program at compile time through proper typed compiler/program interfaces rather than relying primarily on textual macros.
- Capable of rich runtime reflection, to the degree that it makes semantic and implementation sense.
- Coherent enough that compile-time programming, type construction, specialization, constraints, and reflection feel like manifestations of the same underlying model rather than unrelated features.

A useful guiding principle is:

> One language, multiple phases. Runtime code and compile-time code use the same language semantics wherever possible.

---

## 2. Bindings and phases

There are currently four preferred binding forms, representing two independent axes: phase and deep mutability.

```text
let x = expr;          // immutable runtime binding
let mut x = expr;      // mutable runtime binding
comp x = expr;         // immutable compile-time binding
comp mut x = expr;     // mutable compile-time binding
```

The intended conceptual matrix is:

|                | Immutable | Mutable |
|----------------|-----------|---------|
| Runtime        | `let`     | `let mut` |
| Compile time   | `comp`    | `comp mut` |

`mut` controls both whether the binding may be rebound and whether mutation is permitted through that binding. An immutable binding provides deeply immutable access to its value: neither the binding itself nor state reachable through it may be mutated.

For example:

```text
let users = Vec<User>.new();
users.push(user);       // invalid: push requires mutable access
users = other_users;    // invalid: binding is immutable

let mut users = Vec<User>.new();
users.push(user);       // valid
users = other_users;    // valid
```

The same rule applies through nested access paths:

```text
let team = Team.new();
team.members.push(user); // invalid: team provides immutable access

let mut team = Team.new();
team.members.push(user); // valid if members is a mutable field
```

Mutating operations therefore need mutable access to their receiver. The language should not provide general-purpose interior mutation that bypasses an immutable binding. The exact aliasing and borrowing rules needed to preserve deep immutability remain an open question.

Explicit type annotations should work uniformly:

```text
let x: i32 = 10;
let mut y: i32 = 20;
comp N: usize = 32;
comp T: type = SomeType;
```

A normal `let` binding should remain semantically runtime even if an optimizer can constant-fold its initializer. `comp` means compile-time evaluation/availability is part of the language semantics, not merely an optimization.

---

## 3. Anonymous program entities and identity

Structs, functions, enums, modules, and similar entities should be able to exist anonymously as values and then be bound to identifiers.

Example:

```text
comp Vec2 = struct {
    x: i32,
    y: i32,
};
```

The identifier is a binding to a type value; it is not necessarily the intrinsic source of the type's identity.

Two separately evaluated anonymous struct expressions with identical shape should be nominally distinct:

```text
comp Vec2 = struct { x: i32, y: i32 };
comp Pos  = struct { x: i32, y: i32 };

// nominally distinct
Vec2 != Pos
```

But aliases should preserve identity:

```text
comp Vec2 = struct { x: i32, y: i32 };
comp Alias = Vec2;

Vec2 == Alias
```

The language should distinguish nominal identity from structural shape equivalence.

Conceptually:

```text
Vec2 == Pos                    // false: different type identity
Vec2.shape == Pos.shape        // true: same structure/layout description
```

This allows nominal typing while retaining structural introspection and structural constraints where useful.

Names should be treated as symbol/binding metadata rather than necessarily as intrinsic type identity. A type can have zero, one, or many bindings pointing to it.

---

## 4. Types are compile-time values

Types should be first-class compile-time values.

Example:

```text
comp Vec = fn(comp T: type, comp N: usize) -> type {
    return struct {
        data: [N]T;
    };
};

comp Vec4i = Vec(i32, 4);
```

The design currently leans toward not having a separate generic system at all.

Instead:

> What other languages call generics are compile-time functions and mixed-phase functions whose compile-time arguments participate in type construction and specialization.

This should avoid separate concepts such as type parameters, const generics, template parameters, etc. when ordinary compile-time parameters suffice.

---

## 5. Mixed-phase functions

Functions may have both required compile-time parameters and ordinary parameters.

Example:

```text
fn identity(comp T: type, value: T) -> T {
    return value;
}
```

`comp` parameters are required to be known during specialization/typechecking of the function instance.

Ordinary parameters do not have to be known at compile time, but they may still be supplied with compile-time values if the whole call is evaluated at compile time.

This distinction is important:

- `comp` parameter = must be statically known.
- ordinary parameter = may remain dynamic, but is not forbidden from being static.

Example:

```text
fn add(comp T: type, a: T, b: T) -> T {
    return a + b;
}

comp x = add(i32, 2, 3);        // whole function may execute at compile time
let y = add(i32, runtime_a, runtime_b); // specialize on T, execute remaining work at runtime
```

A useful conceptual model is partial evaluation:

```text
fn foo(comp A, B, comp C, D)
```

can be viewed as:

```text
specialize foo with A and C
        ↓
residual function(B, D)
        ↓
execute residual function later
```

Compile-time values may influence runtime computation and types. Runtime-only values may not influence compile-time computation.

Core phase dependency rule:

```text
compile-time  --->  runtime
runtime       -X->  compile-time
```

Compile-time parameters should be able to affect later parameter types and return types:

```text
fn first(comp T: type, values: []T) -> T {
    return values[0];
}
```

Possibly even more reflective forms:

```text
fn get(
    comp S: type,
    comp field: FieldOf(S),
    value: S,
) -> field.type {
    return value.@field(field);
}
```

Function parameter declarations should likely be semantically ordered so later parameter types may depend on earlier compile-time parameters.

Open question: whether compile-time parameters may be interleaved arbitrarily with ordinary parameters in surface syntax. The design currently leans yes, provided dependencies are well-defined.

---

## 6. Specialization and identity

Specializing a mixed function with the same compile-time arguments should conceptually produce the same canonical specialization rather than a fresh incompatible function identity each time.

Example:

```text
fn foo(comp T: type, x: T) -> T { ... }

comp a = foo(i32);
comp b = foo(i32);

// desirable
// a and b refer to the same canonical specialization
```

Likewise, type-producing compile-time functions should generally canonicalize/memoize equivalent instantiations:

```text
comp Vec = fn(comp T: type) -> type {
    return struct { data: *T };
};

comp A = Vec(i32);
comp B = Vec(i32);

// desirable: A == B
```

This differs from evaluating two independent literal struct expressions, which should produce distinct nominal types.

A plausible identity model for specialization is based on:

```text
(original function identity, compile-time argument identities/values)
```

Exact rules remain open.

---

## 7. Bidirectional type inference as constraint solving

Type inference should be bidirectional and should treat calls involving inferred compile-time parameters as ordinary constraint solving, not as a separate generic inference mechanism.

Given:

```text
comp identity = fn(comp T: type, value: T) -> T {
    return value;
};
```

Both of these should work:

```text
let x: i32 = identity(5);
let y = identity("Hi");
```

For the first case, the expected result type helps solve `T`:

```text
expected result: i32
        ↓
identity(...) -> T
        ↓
T = i32
        ↓
literal 5 must be compatible with i32
```

For the second, the argument type determines `T`, which determines the return type.

The important design principle is:

> Compile-time argument inference, result-type inference, literal typing, and ordinary type equality should participate in one solving process.

Explicit compile-time arguments should probably remain available when inference is ambiguous, e.g. some future syntax equivalent to:

```text
identity(T = i64, 5)
```

Exact call syntax is undecided.

---

## 8. Constraints / `where` clauses

Functions should support constraints over compile-time and potentially runtime arguments.

Example shape:

```text
fn get(&self, index: usize) -> ValueType
where {
    index < self.length;
}
{
    ...
}
```

Constraints should be compile-time enforced in the sense that a call is accepted only if the required proposition can be established according to the language's proof/constraint rules.

This can apply both to values and types.

Value example:

```text
fn div(a: i32, b: i32) -> i32
where {
    b != 0;
}
{
    ...
}
```

Then:

```text
let x = div(a, 5); // trivially provable
```

while:

```text
let x = div(a, b); // invalid unless b != 0 is known/proven
```

but flow-sensitive knowledge should make this valid:

```text
if b != 0 {
    let x = div(a, b);
}
```

The condition should add the proposition `b != 0` to the local proof environment.

Type constraint example:

```text
fn sum(comp T: type, values: []T) -> T
where {
    Add(T, T).Output == T;
    Zero(T);
}
{
    ...
}
```

The same solver should ideally handle:

- inferred compile-time parameters,
- type equality,
- interface/trait satisfaction,
- literal compatibility,
- value-level propositions,
- facts learned through control flow.

`where` constraints should participate in inference rather than being checked only after inference completes.

Function-level and case-level `where` clauses have distinct roles:

- A function-level `where` clause is a caller proof obligation. A call is legal
  only when the required proposition can be established at the call site.
- A case-level `where` clause is an ordered dispatch guard. It selects which
  implementation case executes and may be evaluated at runtime when static
  knowledge does not already determine the selected case.

The shared `where` spelling reflects that both forms contribute propositions
to the proof environment, but one restricts the function's domain while the
other partitions its implementation.

The `require` statement introduces a local proof obligation:

```text
require proposition;
```

The compiler must establish the proposition from facts available at that
program point. Failure to prove it is a compile-time error; `require` does not
insert a runtime assertion or failure branch. In this sense it is the
statement-level counterpart of a function-level `where` clause.

For example, a flow fact and an explicit case contract may discharge a local
requirement:

```text
if x < list.length {
    require list.get(x) is Some(let value);
    use(value);
}
```

A binding introduced by a proven pattern proposition is available after the
`require` statement.

The language may also provide `assert` as the runtime-checked counterpart:

```text
assert proposition;
```

Unlike `require`, an unproven `assert` is permitted and emits a runtime check.
The compiler should warn when it can prove that an assertion is always true or
always false. An always-true assertion is redundant; an always-false assertion
guarantees that execution reaching it will fail. Whether these diagnostics are
enabled by default, and whether either condition is elevated to an error,
remain diagnostic-policy questions.

---

## 9. Constraint semantics: proposition vs boolean

There is an important distinction between a runtime/compile-time boolean value and a proposition that the compiler is required to establish.

A future design may introduce a dedicated conceptual `Prop` type or proof obligation model.

For example:

```text
where {
    index < self.length;
}
```

should not necessarily mean "evaluate this boolean right now," because `index` may be runtime data.

Instead it may mean:

> Require evidence that `index < self.length` holds at this program point.

The compiler may discharge constraints in two broad ways:

1. **Evaluation** — all relevant values are compile-time known, so evaluate the predicate.
2. **Proof / symbolic reasoning** — use a deliberately bounded solver and facts from program flow.

The solver should not attempt unrestricted theorem proving or inversion of arbitrary user code.

A reasonable initial proof domain may include:

- type equality,
- interface/trait satisfaction,
- integer equality and inequality,
- constant arithmetic,
- simple ranges,
- logical conjunction/disjunction/negation,
- facts learned from branching,
- perhaps basic algebraic normalization.

If a constraint cannot be established, the compiler should emit a clear diagnostic such as:

```text
error: constraint not proven
    required: index < self.length
```

Open question: whether users can explicitly provide proofs/evidence objects, and if so what their type and syntax should be.

---

## 10. Compile-time control flow

The language should avoid creating unnecessary duplicate syntax for compile-time control flow if phase can be inferred from values.

Potentially:

```text
if T == i32 {
    ...
}
```

where `T` is compile-time known could execute/specialize at compile time automatically.

Likewise:

```text
for field in T.fields() {
    ...
}
```

could become compile-time iteration when the iterable is compile-time known.

Alternative explicit syntax such as `comp if` / `comp for` remains possible, but the current preference is to explore phase propagation first.

The optimizer may still constant-fold ordinary runtime code, but compile-time phase semantics should remain conceptually distinct from optimization.

---

## 11. Ordered function cases and postconditions

A function may be defined as an exhaustive, top-to-bottom sequence of `case`
clauses. Each case may have a `where` guard and an `ensures` postcondition.

```text
comp get = fn(&self, index: usize) -> Option(T) [
    case
    where index < self.length
    ensures return is Some(...) {
        return Some(self.arr[index]);
    }

    case
    where index >= self.length
    ensures return is None {
        return None;
    }
]
```

Cases are considered in source order. Earlier matching cases take precedence
when guards overlap; overlap is legal and may be an intentional form of
priority dispatch. A diagnostic should therefore not warn merely because two
cases partially overlap. It may warn when an earlier case makes an entire
later case provably unreachable.

Case guards are executable dispatch conditions, not proof obligations imposed
on callers. When the caller's facts determine a case statically, the compiler
may select or eliminate cases during specialization. Otherwise, guards are
tested in source order at runtime until one matches.

Case guards must be pure. They may inspect runtime arguments and immutable
state, but may not mutate state, perform I/O, depend on nondeterministic effects,
or otherwise change the program state while dispatching. Purity is necessary
for exhaustiveness checking, ordered reasoning, and for a selected guard to
remain a valid fact inside its case.

Together the cases must be exhaustive. The compiler must prove that their
guards cover every valid input unless the function supplies an unconditional
default case:

```text
case {
    // fallback
}
```

All cases currently share the function's single parameter list. Cases cannot
declare different arguments or act as overloads. Function overloading remains
separate, undecided design space.

Within a case, its `where` condition is available as a fact when checking the
body. Because selection is ordered, the body may also assume that every earlier
case guard was false. For example:

```text
case where x < 0 { ... }
case where x < 10 {
    // x >= 0 and x < 10
}
```

Both `where` and `ensures` are optional. A case without `where` is
unconditional and therefore acts as a default. A function may consist of a
single unconditional case that declares only an `ensures` contract:

```text
fn make_value(...) -> Value [
    case
    ensures return is Valid(...) {
        ...
    }
]
```

`ensures` is compiler-verified documentation about the result of that case,
not an unchecked annotation. Verified postconditions may be exposed to callers
and used by flow-sensitive reasoning. For example, branching on a call result
with `is Some(let value)` may allow the compiler to relate that result to a
case whose postcondition guarantees `Some(...)`.

Only explicitly documented `ensures` clauses form the public logical contract.
The compiler must not expose additional postconditions inferred from the body,
because callers should not acquire dependencies on implementation details that
may change. A case without `ensures` promises no additional facts beyond its
declared types and other explicit contracts.

Case contracts support reasoning in both directions. Known input facts may
identify a case and expose its documented postcondition. Conversely, an
observed result may rule out cases whose explicit postconditions contradict
that observation, allowing the caller to recover facts from the remaining
case guards:

```text
case where index < self.length
ensures return is Some(...) { ... }

case where index >= self.length
ensures return is None { ... }
```

```text
if get(index) is Some(let value) {
    // The None case is impossible, so index < self.length.
}
```

This backward reasoning may use exhaustiveness, ordered effective guards, and
explicit postconditions. It must remain conservative: a case with no relevant
postcondition cannot be eliminated based on facts inferred from its body. If
several cases remain possible, the resulting input knowledge is their
disjunction rather than an arbitrary choice of one case.

Facts learned from ordinary control flow may also select a documented case and
make a result pattern provably irrefutable:

```text
if x < list.length {
    require list.get(x) is Some(let value);
    use(value);
}
```

If `get` documents that `x < list.length` selects a case ensuring a `Some`
result, the requirement is proven and the destructuring binding is available
afterward without an `else` or runtime failure path. This relies only on the
explicit case contract, not on inspection of `get`'s implementation.

Postconditions should eventually be able to describe mutations and relations
between pre-call and post-call state, not only the return value. For example,
the language needs some eventual equivalent of:

```text
ensures self.length == old(self.length) + 1
```

The need for pre-state references is accepted, but `old(...)` is only a
placeholder; its final syntax and interaction with aliasing remain open.

When the function's return type supplies enough context, a case may use an
inferred-return shorthand:

```text
case where index >= self.length => None;
```

Here `=> None` means infer the enclosing return type and return its `None`
case. The explicit form is:

```text
case where index >= self.length => Option(T)::None;
```

This is contextual selection of a case belonging to the `Option(T)` tagged
union, rather than an arbitrary coercion. The exact representation and general
construction syntax of tagged unions remain open.

---

## 12. Pattern conditions and flow-sensitive bindings

`is` and `isnt` may be used for pattern conditions. The currently intended
forms include literal tests and tagged-union destructuring:

```text
if x is 5 {
    ...
}

if x is Some(...) {
    ...
}

if x is Some(let value) {
    // value is bound here
}
```

A binding introduced by a successful pattern is available in the region where
that match is known to hold. An `else` guard can therefore introduce a binding
for the continuation:

```text
x is Some(let value) else {
    return fallback;
}

// value is available here
use(value);
```

Negated pattern syntax participates in the same flow-sensitive reasoning:

```text
if x isnt Some(let value) {
    return;
}

// Every path reaching here established the positive pattern,
// so value is available here.
use(value);
```

This requires definite-assignment analysis based on control-flow exits; merely
writing a binding beneath `isnt` does not make it available on a path where the
positive match has not been established.

If both branches continue, the binding is available only in an explicit
positive `else` branch:

```text
if x isnt Some(let value) {
    handle_absence();
} else {
    // value is available here
    use(value);
}

// value is not available here: both outcomes can reach this point
```

Comparison-like patterns such as `x is > 5`, the broader limits of `is`
syntax, and type-level matching against tagged unions are intentionally left
undecided for now.

---

## 13. Reflection

Compile-time and runtime reflection should expose the same conceptual semantic model wherever meaningful.

Potential reflected concepts include:

```text
Type
Field
Function
EnumCase
Attribute
Module
Declaration
Binding
```

Compile-time reflection may expose richer program/compiler state than runtime reflection.

Runtime reflection should not require inserting an identity field into every struct instance.

Instead, type identity and metadata should live in external/type-level descriptors such as conceptually:

```text
struct TypeInfo {
    id: TypeId,
    name: str,
    size: usize,
    alignment: usize,
    fields: []FieldInfo,
    ...
}
```

Ordinary values retain their declared layout. Dynamic/existential containers can opt into carrying type descriptors if needed.

Runtime metadata should ideally be demand-driven/tree-shakeable so unused reflection information need not bloat binaries.

Possible modes might eventually include full reflection, names-only reflection, minimal reflection, or none.

---

## 14. Compile-time program manipulation

The compiler should expose the program being compiled as a typed semantic object model.

Prefer semantic construction and mutation APIs over raw token/AST rewriting.

Conceptual examples:

```text
comp {
    for decl in program.current_module().declarations() {
        if decl.has_attribute(.derive_eq) {
            derive_eq(decl);
        }
    }
}
```

and:

```text
comp fn derive_hash(target: program.Struct) {
    comp impl = program.create_impl(Hash, target.type());
    impl.add_method(...);
    program.add(impl);
}
```

Potential abstraction levels:

```text
syntax.Expr
syntax.Decl

sem.Type
sem.Function
sem.Struct

program.Module
program.Binding
program.Declaration
```

Raw syntax generation/quasiquoting may exist as a lower-level mechanism, but semantic construction should be the normal path where possible.

Program entities and bindings should be distinct concepts: multiple symbols may point to one semantic entity.

---

## 15. Compiler staging / mutation barriers

Because compile-time code may modify the program, compilation needs explicit staging rules to avoid incoherent self-modification.

A possible staged model:

```text
1. parse source declarations
2. create initial program graph
3. run declaration-generation compile-time code
4. resolve generated declarations
5. run semantic metaprograms
6. freeze relevant program structure
7. lower / specialize / optimize
8. generate code
```

The exact pipeline is open, but the language should avoid unconstrained mutation where generated code can endlessly invalidate arbitrary earlier semantic decisions.

The compiler may need to act more like an incremental interpreter + typechecker + semantic program database than a simple one-way compiler pipeline.

---

## 16. Compile-time effects and reproducibility

Because nearly arbitrary code may execute at compile time, effects need deliberate semantics.

Potential effect/capability categories include:

- filesystem,
- environment variables,
- networking,
- process execution,
- compiler/program mutation,
- allocation,
- runtime-only resources.

Compile-time effects should ideally be visible to the build system so dependencies are tracked rather than hidden.

Exact syntax/effect system is undecided.

---

## 17. Compile-time termination and diagnostics

Compile-time execution is not guaranteed to terminate.

The implementation should provide useful safeguards and diagnostics such as configurable step/resource limits and compile-time evaluation stack traces.

Example diagnostic shape:

```text
error: compile-time evaluation exceeded configured step limit

evaluation stack:
    generate_parser()
    build_state_machine()
    ...
```

---

## 18. Current design principles

The current design can be summarized with these principles:

1. One language, multiple execution phases.
2. Four explicit binding forms: `let`, `let mut`, `comp`, `comp mut`; immutability is deep, so mutation through an immutable binding is forbidden.
3. Types are first-class compile-time values.
4. No dedicated generic system is necessary if compile-time functions and mixed functions cover the space cleanly.
5. `comp` parameters are required-static parameters; normal parameters may still be used during compile-time evaluation when values are available.
6. Mixed functions specialize through partial evaluation.
7. Specialization should generally be canonical for the same compile-time arguments.
8. Anonymous type/function/etc. expressions create semantic entities with identity independent of identifier names.
9. Separate nominal identity from structural shape equality.
10. Type inference should be bidirectional and constraint-driven.
11. `where` clauses express propositions the compiler must establish, not merely runtime assertions.
12. Flow-sensitive control flow should contribute facts to the proof environment.
13. Type constraints and value constraints should share the same conceptual solver.
14. Compile-time reflection and runtime reflection should share one semantic model where sensible.
15. Runtime reflection metadata belongs to types/program metadata, not necessarily to every instance.
16. Compile-time metaprogramming should primarily manipulate typed semantic program objects rather than strings/tokens.
17. Program mutation needs explicit staging or stabilization rules.
18. Compile-time effects and resource usage need explicit, diagnosable semantics.
19. Multi-case functions select exhaustive cases from top to bottom.
20. `ensures` clauses are verified postconditions that may contribute facts to callers.
21. Context may infer a tagged-union case in an expression such as `=> None` from the enclosing return type.
22. `is` and `isnt` patterns introduce bindings wherever control flow proves that the positive pattern holds.
23. Later ordered cases inherit the negation of all earlier case guards.
24. Only explicit `ensures` clauses are public logical contracts; body-inferred facts are not exposed to callers.
25. Case contracts permit conservative backward reasoning from observed results to possible case guards.
26. Function-level `where` clauses are caller proof obligations; case-level `where` clauses are executable ordered dispatch guards.
27. Case guards are pure, and flow-sensitive facts may make result patterns provably irrefutable through explicit case contracts.
28. `require` states a local compile-time proof obligation and exposes bindings introduced by a proven pattern.
29. Runtime `assert` may accept unproven propositions and should warn when its condition is provably always true or false.

---

## 19. Important open questions

Codex should continue exploring these rather than assuming answers:

### Syntax

- Final function declaration syntax.
- Whether anonymous functions use `fn(...) {}` exactly as shown.
- Named argument syntax, especially for explicit compile-time parameters.
- Whether compile-time control flow needs explicit syntax (`comp if`, `comp for`) or is inferred from phase.
- Exact syntax for type/value reflection and field access by reflected field objects.
- The full scope of `is` patterns, including whether comparison-like forms such as `x is > 5` should exist.
- Tagged-union declaration, construction, qualification, and type-level matching syntax.
- Final syntax for unconditional/default function cases.

### Type identity

- Exact identity model for anonymous type expressions.
- Exact canonicalization rules for compile-time function calls that return types.
- Equality semantics for compile-time values that participate in specialization keys.

### Function specialization

- Whether partial specialization is always first-class as a function value.
- Whether compile-time parameters may appear after ordinary parameters.
- How closures/captured values interact with specialization and phase.
- ABI/codegen model for mixed functions.

### Inference

- Literal typing rules.
- Ambiguity handling.
- How expected return types flow backward through calls.
- How overloads/interfaced functions interact with compile-time argument inference.

### Constraints / proofs

- Whether there is an explicit `Prop` type or only internal proof obligations.
- Exact solver scope.
- How user-defined constraints become solver-visible without requiring arbitrary theorem proving.
- Whether runtime checks can explicitly discharge a proof obligation.
- Whether proof/evidence values can be passed explicitly.
- How verified case postconditions are represented and propagated across calls.
- How exhaustiveness is proven and how unreachable cases are diagnosed.
- Syntax and semantics for referring to pre-call state in mutation postconditions.
- How mutation invalidates previously established facts.
- How aliasing affects proofs over object fields and lengths.

### Reflection

- Which metadata is guaranteed at runtime.
- How metadata retention/tree-shaking works.
- Whether reflected functions are invokable dynamically and under what type safety model.
- Whether source-level information is available at runtime.

### Program mutation

- Exact stages in which program mutation is legal.
- Whether transformations are transactional.
- How generated declarations receive source/debug locations and names.
- Hygiene/name resolution model.

### Effects

- Compile-time filesystem/network/process permissions.
- Reproducible build semantics.
- Whether effects are tracked with an explicit type/effect system or compiler capability model.

---

## 20. Suggested next design topic

The next useful design pass should focus on the constraint/proof system and mixed-function semantics together.

In particular, work through concrete examples such as:

```text
fn identity(comp T: type, value: T) -> T
```

```text
fn get(&self, index: usize) -> ValueType
where {
    index < self.length;
}
```

```text
fn div(a: i32, b: i32) -> i32
where {
    b != 0;
}
```

```text
fn sum(comp T: type, values: []T) -> T
where {
    Add(T, T).Output == T;
    Zero(T);
}
```

Questions to resolve include:

- What exactly is generated during call-site constraint solving?
- Which constraints are solved by evaluation versus symbolic proof?
- What facts survive through mutation?
- What is the error model when a constraint cannot be proven?
- Can a programmer explicitly convert a runtime check into proof evidence?
- How does bidirectional inference interact with unresolved `where` constraints?
- When does specialization happen, and when is a mixed function considered a concrete function value?

---

## 21. Instructions for future iteration

When iterating on this language design:

- Prefer a small number of orthogonal mechanisms over many special-case features.
- Do not introduce conventional generics, templates, traits, macros, or constexpr-like subsystems unless the existing compile-time value/function/reflection model genuinely cannot express the requirement cleanly.
- Always test a proposed feature against both compile-time and runtime use.
- Keep type identity, symbol naming, structural shape, and runtime layout conceptually separate.
- Treat compiler diagnostics and predictability as first-class design constraints.
- When proposing syntax, explain the semantic model first.
- Use concrete examples and edge cases before declaring a design settled.
- Mark speculative decisions as open rather than silently converting them into firm language rules.
