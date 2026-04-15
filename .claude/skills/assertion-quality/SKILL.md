---
name: assertion-quality
description: "Analyzes the variety and depth of assertions across test suites in any supported language. Use when the user asks to evaluate assertion quality, find shallow testing, identify tests with only trivial assertions, measure assertion diversity, or audit whether tests verify different facets of correctness. Produces metrics and actionable recommendations. Supports Rust and .NET (MSTest, xUnit, NUnit, TUnit). Also detects implicit assertions, panic/exception testing, mock verification, and framework-specific patterns. DO NOT USE FOR: writing new tests, detecting general anti-patterns, or fixing existing assertions — help the user directly with those."
---

# Assertion Diversity Analysis

Analyze test code to measure how varied and meaningful the assertions are. Produce a metrics report that reveals whether tests verify different facets of correctness — not just "output equals X" but also error paths, structure, state transitions, side effects, and invariants.

## Why Assertion Diversity Matters

Low assertion diversity signals shallow testing. Tests may pass while bugs hide in unasserted logic. Common symptoms across all languages:

| Problem | Symptom | Consequence |
|---------|---------|-------------|
| Trivial assertions | Only checking non-null/non-error | Test passes but doesn't verify correctness |
| Single-value obsession | Always check one field or return value | Bugs in unchecked fields slip through |
| No error path testing | Never test failure cases | Error handling is untested |
| No negative assertions | Never check what shouldn't happen | Regressions sneak in through false positives |
| No state checks | Don't verify object state changes | Missed side-effects or lifecycle issues |
| No structural checks | Only assert top-level value | Bugs in nested objects go unnoticed |
| Assertion-free tests | Tests that call but don't verify | Code coverage lies; false security |

## When to Use

- User asks to evaluate assertion quality or depth
- User asks "are my tests actually testing anything meaningful?"
- User wants to know if test assertions are too shallow or trivial
- User asks for assertion diversity metrics or analysis
- User suspects tests give false confidence despite passing

## When Not to Use

- User wants to write new tests (help them directly)
- User wants to detect anti-patterns beyond assertions (help them directly)
- User wants to fix or rewrite assertions (help them directly)
- User asks about code coverage percentages (out of scope — this analyzes assertion quality, not line coverage)

## Language Detection

Detect the language from the files provided and load the appropriate reference:

| Language | File patterns | Reference |
|----------|--------------|-----------|
| **Rust** | `*.rs` files, `Cargo.toml` | Read `references/rust.md` |
| **.NET** | `*.cs` files, `*.csproj`, `*.sln` | Read `references/dotnet.md` |

If the language is ambiguous, ask the user. If the test files contain a mix of languages, analyze each language separately and produce a combined report.

**After detecting the language, read the corresponding reference file before proceeding.** The reference contains framework-specific assertion patterns, classification examples, and calibration rules that are essential for accurate analysis.

## Inputs

| Input | Required | Description |
|-------|----------|-------------|
| Test code | Yes | One or more test files, a test directory, or inline test modules |
| Production code | No | The code under test, to evaluate whether assertions cover the important behaviors |

## Universal Assertion Categories

These 12 categories apply across all languages. Each language reference file maps framework-specific syntax to these categories.

| # | Category | What it verifies |
|---|----------|-----------------|
| 1 | **Equality** | Return value matches expected |
| 2 | **Inequality** | Values differ |
| 3 | **Boolean** | A condition holds |
| 4 | **Null/None/Nil** | Presence or absence of a value |
| 5 | **Error/Exception/Panic** | Error handling behavior |
| 6 | **Type/Pattern** | Runtime type correctness or structural pattern match |
| 7 | **String** | Text content and format |
| 8 | **Collection** | Collection contents, length, and structure |
| 9 | **Comparison** | Ordering and magnitude |
| 10 | **Approximate** | Floating-point or tolerance-based |
| 11 | **Negative** | What should NOT happen |
| 12 | **State/Side-effect** | State transitions, mock verification, side effects |
| 13 | **Structural/Deep** | Nested object correctness, serialization round-trips |
| 14 | **Implicit** | Framework or language-specific implicit assertions (e.g., `.unwrap()` in Rust, implicit mock verification on dispose in .NET) |

A single assertion can belong to multiple categories. The language reference file contains examples of multi-category classifications.

## Workflow

### Step 1: Gather the test code

Read all test files the user provides. If the user points to a directory or project, scan for test files using the framework-specific markers described in the language reference.

### Step 2: Classify every assertion

For each test method/function, identify all assertions and classify them into the categories above. **Consult the language reference file** for framework-specific assertion syntax and classification rules.

Key principles (apply across all languages):
- Count both explicit assertions (framework macros/methods) and implicit assertions (language-specific patterns from the reference)
- A single assertion can belong to multiple categories
- Record whether each test is "implicit-only" (relies solely on implicit assertions with no explicit ones)

### Step 3: Compute metrics

Calculate these metrics for the test suite:

#### Per-test metrics
- **Assertion count**: Explicit assertions + implicit assertions
- **Assertion categories**: Which categories each test uses
- **Implicit-only flag**: Whether the test has no explicit assertions

#### Suite-wide metrics
- **Total tests**: Count of all test functions/methods
- **Average assertions per test**: Total assertions / total tests
- **Assertion type spread**: Distinct categories used / 14
- **Tests with zero assertions**: No assertions at all (not even implicit)
- **Implicit-only tests**: Only implicit assertions, no explicit ones
- **Tests with only trivial assertions**: Every assertion is trivial (see calibration)
- **Tests with error/exception/panic assertions**: Count and percentage
- **Tests with pattern/type assertions**: Count and percentage
- **Tests with negative assertions**: Count and percentage (target: >= 10%)
- **Tests with state/side-effect assertions**: Count and percentage
- **Single-category tests**: Tests using only one assertion category

### Step 4: Apply calibration rules

Before reporting, apply these universal calibration rules plus any language-specific rules from the reference:

- **Trivial means truly trivial.** A null/none check *before* a value assertion is a guard, not trivial. Only flag a test as trivial if it has no meaningful value verification.
- **Boolean assertions on meaningful conditions are not trivial.** Checking a specific property or method return is meaningful. Checking a literal `true` is trivial.
- **Error/exception/panic tests are inherently low-assertion-count.** The error verification *is* the assertion. Don't penalize these for low count.
- **Property-based tests may have single assertions.** Frameworks like `proptest` (Rust) or `FsCheck` (.NET) generate hundreds of inputs — one assertion per property is fine.
- **Don't conflate diversity with volume.** 20 equality checks = high volume, low diversity. One equality + one error + one pattern match = low volume, good diversity.
- **Consider the code under test.** A pure function returning a simple value legitimately needs only equality checks. A stateful object with error paths needs broader categories.
- **If assertions are well-diversified, say so.** A positive report is valid.

### Step 5: Report findings

Present the analysis in this structure:

#### 1. Summary Dashboard
Quick-reference table of key metrics with assessments (Good/Moderate/Low/Concerning).

#### 2. Category Breakdown
For each assertion category: how many tests use it, representative examples, whether overused or underused.

#### 3. Gap Analysis
Based on the production code (if available), identify:
- Functions/methods with untested error paths
- State-changing operations with no post-mutation assertions
- Return types that are only shallowly checked (e.g., checking `is_ok()` without inspecting the value)
- Collections returned but never checked for contents
- Public API boundaries where only the happy path is tested

#### 4. Recommendations
Prioritized list with:
- Which tests would benefit most from additional assertion types
- Which categories are missing and why they matter
- **Concrete code snippets** using the actual types from the codebase

#### 5. Implicit-only tests
List each test relying solely on implicit assertions, with its name and apparent intent.

#### 6. Assertion-free tests
List any tests with no verification at all.

## Validation Checklist

- [ ] Correct language reference was loaded
- [ ] Every assertion classified into at least one category
- [ ] Implicit assertions identified (language-specific)
- [ ] Error/exception/panic tests not penalized for low count
- [ ] Property-based tests not penalized for single assertions
- [ ] Trivial tests correctly identified (not over-flagged)
- [ ] Recommendations are concrete with code snippets
- [ ] If diversity is good, the report says so
- [ ] Metrics add up correctly
