# Rust Assertion Reference

## Test Framework Detection

Scan for these markers to identify test code:
- `#[test]` — standard test functions
- `#[rstest]` — parameterized tests (rstest crate)
- `#[tokio::test]` — async tests
- `proptest! { }` — property-based test blocks
- `#[cfg(test)] mod tests` — inline test modules
- `tests/` directory — integration tests

## Assertion Classification

### Equality
| Pattern | Example |
|---------|---------|
| `assert_eq!(a, b)` | `assert_eq!(result, 42)` |
| `assert_eq!(a, b, "msg")` | `assert_eq!(name, "alice", "wrong name")` |
| `pretty_assertions::assert_eq!` | Same semantics, better diff output |

### Inequality
| Pattern | Example |
|---------|---------|
| `assert_ne!(a, b)` | `assert_ne!(id, 0)` |

### Boolean
| Pattern | Example |
|---------|---------|
| `assert!(expr)` | `assert!(path.exists())` |
| `assert!(expr, "msg")` | `assert!(list.is_sorted(), "not sorted")` |
| `assert!(x.is_valid())` | Meaningful property check — NOT trivial |

**Trivial boolean assertions** (flag these):
- `assert!(true)`
- `assert!(!false)`

### Null/None (Result/Option)
| Pattern | Example | Notes |
|---------|---------|-------|
| `assert!(opt.is_some())` | `assert!(config.is_some())` | Trivial if inner value never inspected |
| `assert!(opt.is_none())` | `assert!(cache.get("k").is_none())` | Negative + Null |
| `assert!(result.is_ok())` | `assert!(parse("42").is_ok())` | Trivial if Ok value never inspected |
| `assert!(result.is_err())` | `assert!(parse("bad").is_err())` | Negative + Error |

### Error/Panic
| Pattern | Example | Notes |
|---------|---------|-------|
| `#[should_panic]` | `#[should_panic(expected = "overflow")]` | Panic + optionally String |
| `.unwrap_err()` | `let e = result.unwrap_err();` | Guard that confirms Err |
| `assert!(result.is_err())` | See above | Also Negative |
| `let Err(e) = result else { panic!() };` | Destructuring error extraction | Pattern matching + Error |

### Type/Pattern Matching
| Pattern | Example |
|---------|---------|
| `assert_matches!(val, Pattern)` | `assert_matches!(err, MyError::NotFound { .. })` |
| `assert!(matches!(val, Pattern))` | `assert!(matches!(resp, Response::Ok(_)))` |
| `let Variant(inner) = val else { panic!() };` | Destructuring with panic fallback |

Pattern matching assertions are typically **high quality** — they destructure and verify structure, not just a boolean condition.

### String
| Pattern | Example |
|---------|---------|
| `assert!(s.contains("x"))` | `assert!(msg.contains("not found"))` |
| `assert!(s.starts_with("x"))` | `assert!(path_str.starts_with("./"))` |
| `assert!(s.ends_with("x"))` | `assert!(filename.ends_with(".rs"))` |
| `assert_eq!(format!("{err}"), "msg")` | Display trait verification |
| `#[should_panic(expected = "msg")]` | Also Error/Panic |

### Collection
| Pattern | Example |
|---------|---------|
| `assert!(v.is_empty())` | `assert!(errors.is_empty())` |
| `assert!(!v.is_empty())` | Also Negative |
| `assert!(v.contains(&x))` | `assert!(names.contains(&"alice"))` |
| `assert_eq!(v.len(), n)` | `assert_eq!(results.len(), 3)` |
| `assert!(iter.any(\|x\| ...))` | Iterator-based checks |
| `assert!(iter.all(\|x\| ...))` | Universal quantifier |

### Comparison
| Pattern | Example |
|---------|---------|
| `assert!(x > y)` | `assert!(elapsed > Duration::ZERO)` |
| `assert!(x >= y)` | `assert!(count >= min_expected)` |
| `assert!(x < y)` | `assert!(size < MAX_SIZE)` |

### Approximate
| Pattern | Example |
|---------|---------|
| `assert!((x - y).abs() < eps)` | `assert!((result - 3.14).abs() < 0.01)` |
| `approx::assert_relative_eq!` | `assert_relative_eq!(a, b, epsilon = 1e-6)` |
| `approx::assert_abs_diff_eq!` | `assert_abs_diff_eq!(a, b, epsilon = 0.1)` |

### Negative
| Pattern | Example | Notes |
|---------|---------|-------|
| `assert_ne!(a, b)` | `assert_ne!(id, 0)` | Also Inequality |
| `assert!(!condition)` | `assert!(!list.contains(&x))` | Negated boolean |
| `assert!(result.is_err())` | See Error section | Also Error |
| `assert!(opt.is_none())` | See Null section | Also Null |
| `assert!(!path.exists())` | Filesystem absence check | |

### State/Side-effect
| Pattern | Example |
|---------|---------|
| Post-mutation field check | `obj.mutate(); assert_eq!(obj.field, new_val);` |
| `mockall` expectation | `mock.expect_save().times(1).returning(\|_\| Ok(()));` |
| Before/after comparison | `let before = state.count(); op(); assert_eq!(state.count(), before + 1);` |

**`mockall` note:** Mock expectations (`.expect_*().times().returning()`) are verified when the mock is dropped. The setup IS the assertion — it's just deferred. Classify these as State/Side-effect.

### Structural/Deep
| Pattern | Example |
|---------|---------|
| Nested field access | `assert_eq!(result.inner.child.value, expected)` |
| Debug format comparison | `assert_eq!(format!("{:?}", obj), expected_debug)` |
| Serialization round-trip | `let json = serde_json::to_string(&obj)?; let rt: T = serde_json::from_str(&json)?; assert_eq!(obj, rt);` |
| `insta::assert_snapshot!` | Snapshot testing — high quality structural assertion |
| `insta::assert_json_snapshot!` | JSON snapshot |

### Implicit Assertions
| Pattern | Example | Notes |
|---------|---------|-------|
| `.unwrap()` in test body | `let val = result.unwrap();` | Panics on Err/None — de facto assertion |
| `.expect("reason")` in test body | `let cfg = load().expect("should load");` | Same as unwrap with message |
| `?` in Result-returning test | `fn test() -> Result<()> { let v = op()?; ... }` | Idiomatic error propagation — equivalent to unwrap |

**Important:** Only count `.unwrap()`/`.expect()` in the *test function body*, not in the production code called by the test. And `?` in `Result`-returning tests is idiomatic Rust — don't flag it as concerning.

## Calibration Rules (Rust-Specific)

1. **`.unwrap()` chains leading to an explicit assertion are guards, not implicit-only.** `let v = r.unwrap(); assert_eq!(v, 42);` — the unwrap is a guard, the `assert_eq!` is the assertion.

2. **`#[should_panic]` tests are complete with zero `assert!` macros.** The panic attribute IS the assertion.

3. **`proptest!` blocks with a single `assert!` are fine.** The framework generates hundreds of inputs per run.

4. **`Result`-returning tests using `?` are idiomatic.** Don't flag `?` as a concern — it's Rust's standard pattern for test error propagation.

5. **`rstest` parameterized tests run the same assertion body multiple times.** Count assertion diversity per test function, not per parameter set.

6. **`mockall` expectations are deferred assertions.** `.expect_foo().times(1)` is verified on drop. Don't flag mock-heavy tests as assertion-free.

## Multi-Category Examples

| Code | Categories |
|------|-----------|
| `assert!(result.is_err())` | Null/None + Negative |
| `assert_ne!(vec.len(), 0)` | Inequality + Collection |
| `#[should_panic(expected = "invalid")]` | Error/Panic + String |
| `assert!(matches!(err, MyError::NotFound { .. }))` | Pattern + Negative |
| `assert!(!names.contains(&"removed"))` | Negative + Collection |
| `assert_eq!(format!("{err}"), "msg")` | Equality + String |
