# init4 Library Best Practices

## High-level recommendations

Write code as if you were an extremely opinionated style enforcer. Prioritize
matching local patterns and conventions over personal preferences. Avoid
nesting and branching.

We care deeply about code quality in library and binary code. Quality standards
may be relaxed in test code and scripts. Tests should still be human-readable.

We write libraries for ourselves and for others to use. Prioritize usability,
clarity, and maintainability. Favor small, focused functions and types. Strive
for consistency in naming, patterns, and API design across the codebase.

NEVER add incomplete function implementations. `TODO`s in the code and comments
should be related to performance improvements, refactoring, or non-critical
features only. A function that contains a `TODO` for its core logic is not
implemented.

## Testing, linting, formatting and Workflow

When working on a code in a specific crate, it's best to run tests, linters,
and formatters for that crate only, to save time and resources. Always lint
before running testing. Be aware that integration tests may be costly in terms
of time, and avoid running them unnecessarily.

Here are the preferred commands:

```
# run tests (use the -p flag to specify a particular crate)
cargo clippy -p <crate_name> --all-features --all-targets
cargo clippy -p <crate_name> --no-default-features --all-targets
cargo t -p <crate_name>
```

The formatter can be run globally, as it is fast enough:

```
cargo +nightly fmt
```

1. Format.
2. Lint.
3. Test.

## Code Style

### API design

Prefer small, focused functions that do one thing well. Break complex logic
into smaller helper functions. Favor composition over inheritance. Use traits
to define shared behavior. Prefer enums for variants over boolean flags.

When designing APIs, aim for clarity and ease of use. Use descriptive names
for functions, types, and variables. Avoid abbreviations unless they are widely
understood. Strive for consistency in naming conventions and API patterns across
the codebase.

When designing public APIs, consider the following principles:

- Minimize types the user needs to understand to use the API effectively.
- Minimize generics and associated types exposed to the user by providing
  sane defaults and concrete types where possible.
- Generics should disappear from the primary user-facing API wherever
  possible.

When writing traits, include an implementation guide that describes the intended
use cases, design rationale, and any constraints or requirements for
implementers.

When writing complex structs, include builder-pattern instantiation. Ask if you
are unclear whether a struct is complex enough to warrant a builder.

### Imports

Imports should not be separated by empty lines. Remove empty lines and let the
formatter alphabetize them. Ensure that imports are grouped by crate, with
`crate::` imports first and all imports merged.

```
// Avoid:
use std::collections::HashMap;

use other_crate::SomeType;
use other_crate::AnotherType;

use crate::MyType;

// Preferred:
use crate::MyType;
use other_crate::{AnotherType, SomeType};
use std::collections::HashMap;
```

### Comments

Write rust doc for all public items. Use `///` for single line comments and
`//!` for module-level comments. Write example usage tests where appropriate

Simple functions should be explained entirely by the rustdoc. In more
complex cases, use inline comments `//` to explain non-obvious parts of the
implementation. When writing complex code, annotate with comments to explain
the logic. Comments should be concise, and explain the design rational, and any
conditions of use that must be enforced. When writing complicated code, these
should occur about every 5-10 lines

When writing rustdoc identify the primary API types and include example tests
to show high-level API usage:

- be aware that tests are run in doc test modules, so imports may be necessary.
- prefer concise examples that focus on the key usage patterns.
- keep examples short and to the point.
- add inline comments `//` to explain non-obvious parts.
- hide unnecessary scaffolding by prepending the line with a `#` according to
  rustdoc convention. See this example for hidden lines:

```rust
//! # use trevm::{revm::database::in_memory_db::InMemoryDB, {TrevmBuilder}};
//! # fn t<C: Cfg, B: Block, T: Tx>(cfg: &C, block: &B, tx: &T) {
//! TrevmBuilder::new()
//!     .with_db(InMemoryDB::default())
//!     .build_trevm()
//!     .fill_cfg(cfg)
//!     .fill_block(block)
//!     .run_tx(tx);
//! # }
```

### Handling Options and Results

We prefer a terse, functional style. Avoid unnecessary nesting. Avoid unnecessary
closures in functional chains

```rust
// NEVER:
if let Some(a) = option {
    Thing::do_something(a);
} else {
    return;
}

// Preferred styles:
option.map(Thing::do_something);

let Some(a) = option else {
    return;
};
Thing::do_something(a);
```

Strongly prefer functional combinators like `map`, `and_then`, `filter`, etc.
over imperative control flow.

### Test style

Be concise. Avoid unnecessary setup/teardown. Don't bother checking options and
results before unwrapping them. Use `unwrap` or `expect` directly.

```rust
// NEVER:
let result = some_function();
assert!(result.is_ok());
let value = result.unwrap();

// Preferred:
let value = some_function().unwrap();
```

Tests should fail fast and panic rather than return error types.
