## Coding Style

### Error Handling
- Use `unwrap()` instead of `?` or `match` where the value is guaranteed by an invariant.
  - Validate invariants before values reach the function arguments.
  - Keep `Result` for external boundaries and recoverable errors.
  - If a critical invariant breaks, fail fast instead of hiding the bug.

### Type Safety
- Prefer newtypes over raw indices where practical.
- Instead of passing unrelated `usize` values around, define dedicated index or id types:
```rust
struct SensorId(pub usize);
struct SampleIdx(pub usize);
```
- Use newtypes to prevent mixing different domains at compile time.
- Avoid `impl Deref` for newtypes; prefer explicit `.0` access.

### Avoid Option As A Crutch
- Do not use `Option` as a placeholder for "not sure yet"; it hides bugs.
- Default values like `0` or an empty string are worse when they silently mask missing data.
- Prefer `unwrap()` for contract-style programming when the value must exist.
- Use `Option` only when absence is semantically meaningful and unavoidable.

### Comments
- Comment invariants, preconditions, non-obvious properties, and tricky algorithms generously.
- If a function relies on a contract, document the contract near the code that depends on it.
- For formulas, indexing tricks, timing assumptions, and hardware sequencing, explain why the code is shaped that way.
- Multi-line comments are fine when they make the code easier to audit.

### Re-exports
- Avoid broad re-exports with `pub use`.
- Import types directly from their source modules so call sites show where things live.
- Prefer explicit paths over hidden coupling between modules.

### Code Reuse
- Do not duplicate an existing algorithm or hardware setup pattern.
- Before writing a new helper, search the codebase for an existing function or module to extend.
- Extend existing structures and functions instead of copying them with small variations.
