# .NET Assertion Reference

## Test Framework Detection

Scan for these markers to identify test code:
- `[TestMethod]` — MSTest
- `[Fact]`, `[Theory]` — xUnit
- `[Test]`, `[TestCase]` — NUnit
- `[Test]`, `[Arguments]` — TUnit
- `using Microsoft.VisualStudio.TestTools.UnitTesting;` — MSTest
- `using Xunit;` — xUnit
- `using NUnit.Framework;` — NUnit
- `using TUnit;` — TUnit

## Assertion Classification

### Equality
| Framework | Pattern | Example |
|-----------|---------|---------|
| MSTest | `Assert.AreEqual(expected, actual)` | `Assert.AreEqual(42, result)` |
| xUnit | `Assert.Equal(expected, actual)` | `Assert.Equal("alice", name)` |
| NUnit | `Assert.That(actual, Is.EqualTo(expected))` | `Assert.That(count, Is.EqualTo(3))` |
| TUnit | `await Assert.That(actual).IsEqualTo(expected)` | `await Assert.That(name).IsEqualTo("alice")` |
| FluentAssertions | `.Should().Be(expected)` | `result.Should().Be(42)` |

### Inequality
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.AreNotEqual(unexpected, actual)` |
| xUnit | `Assert.NotEqual(unexpected, actual)` |
| NUnit | `Assert.That(actual, Is.Not.EqualTo(unexpected))` |
| TUnit | `await Assert.That(actual).IsNotEqualTo(unexpected)` |

### Boolean
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.IsTrue(condition)`, `Assert.IsFalse(condition)` |
| xUnit | `Assert.True(condition)`, `Assert.False(condition)` |
| NUnit | `Assert.That(condition, Is.True)` |
| TUnit | `await Assert.That(condition).IsTrue()` |

**Trivial boolean assertions** (flag these):
- `Assert.IsTrue(true)` / `Assert.True(true)`
- `Assert.IsFalse(false)` / `Assert.False(false)`

**Not trivial:**
- `Assert.IsTrue(result.IsValid)` — checks a meaningful property

### Null/None
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.IsNull(obj)`, `Assert.IsNotNull(obj)` |
| xUnit | `Assert.Null(obj)`, `Assert.NotNull(obj)` |
| NUnit | `Assert.That(obj, Is.Null)`, `Assert.That(obj, Is.Not.Null)` |
| TUnit | `await Assert.That(obj).IsNull()`, `await Assert.That(obj).IsNotNull()` |

**Calibration:** `Assert.IsNotNull(result)` alone is trivial. `Assert.IsNotNull(result)` followed by `Assert.AreEqual(expected, result.Value)` is NOT trivial — the null check is a guard before the real assertion.

### Error/Exception
| Framework | Pattern | Example |
|-----------|---------|---------|
| MSTest | `Assert.ThrowsException<T>(() => ...)` | `Assert.ThrowsException<ArgumentNullException>(() => svc.Process(null))` |
| MSTest | `Assert.ThrowsExceptionAsync<T>(async () => ...)` | Async variant |
| xUnit | `Assert.Throws<T>(() => ...)` | `Assert.Throws<InvalidOperationException>(() => obj.Start())` |
| xUnit | `Assert.ThrowsAsync<T>(async () => ...)` | Async variant |
| NUnit | `Assert.That(() => ..., Throws.TypeOf<T>())` | `Assert.That(() => op(), Throws.TypeOf<ArgumentException>())` |
| NUnit | `Assert.Throws<T>(() => ...)` | `Assert.Throws<InvalidOperationException>(() => obj.Do())` |
| TUnit | `await Assert.That(() => ...).ThrowsException()` | |
| FluentAssertions | `.Should().Throw<T>()` | `action.Should().Throw<ArgumentException>()` |

**Calibration:** Exception assertions are inherently low-assertion-count. The exception test IS the assertion. Don't penalize these for having only one `Assert` call.

### Type/Pattern
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.IsInstanceOfType(obj, typeof(T))` |
| xUnit | `Assert.IsType<T>(obj)`, `Assert.IsAssignableFrom<T>(obj)` |
| NUnit | `Assert.That(obj, Is.TypeOf<T>())`, `Assert.That(obj, Is.InstanceOf<T>())` |

Also includes C# pattern matching in assertions:
```csharp
Assert.IsTrue(result is SuccessResult { Value: > 0 });
var success = Assert.IsType<SuccessResult>(result);
Assert.Equal(expected, success.Value);
```

### String
| Framework | Pattern |
|-----------|---------|
| MSTest | `StringAssert.Contains(str, substring)`, `StringAssert.StartsWith(str, prefix)` |
| xUnit | `Assert.Contains(substring, str)`, `Assert.StartsWith(prefix, str)` |
| NUnit | `Assert.That(str, Does.Contain(substring))`, `Assert.That(str, Does.StartWith(prefix))` |
| All | `Assert.That(str, Does.Match(regex))` / `Assert.Matches(regex, str)` |

### Collection
| Framework | Pattern |
|-----------|---------|
| MSTest | `CollectionAssert.Contains(collection, item)`, `CollectionAssert.AreEqual(expected, actual)` |
| xUnit | `Assert.Contains(item, collection)`, `Assert.Empty(collection)`, `Assert.Single(collection)` |
| xUnit | `Assert.All(collection, item => ...)` |
| NUnit | `Assert.That(collection, Has.Member(item))`, `Assert.That(collection, Is.Empty)` |
| NUnit | `Assert.That(collection, Has.Count.EqualTo(n))` |
| FluentAssertions | `.Should().Contain(item)`, `.Should().HaveCount(n)`, `.Should().BeEmpty()` |

### Comparison
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.IsTrue(x > y)` (via boolean) |
| xUnit | `Assert.InRange(value, low, high)` |
| NUnit | `Assert.That(x, Is.GreaterThan(y))`, `Assert.That(x, Is.LessThan(y))` |
| NUnit | `Assert.That(x, Is.InRange(low, high))` |

### Approximate
| Framework | Pattern |
|-----------|---------|
| MSTest | `Assert.AreEqual(expected, actual, delta)` |
| NUnit | `Assert.That(actual, Is.EqualTo(expected).Within(tolerance))` |
| xUnit | `Assert.Equal(expected, actual, precision)` (for `decimal`) |

### Negative
Any assertion that verifies what should NOT happen:
- `Assert.AreNotEqual` / `Assert.NotEqual`
- `Assert.DoesNotContain` / `Does.Not.Contain`
- `Assert.DoesNotThrow` / `Assert.That(() => ..., Throws.Nothing)`
- `Assert.IsNotNull` (when checking absence is the point, not a guard)
- `CollectionAssert.DoesNotContain`
- `.Should().NotBe()`, `.Should().NotContain()`, `.Should().NotThrow()`

### State/Side-effect
| Pattern | Example |
|---------|---------|
| Property check after mutation | `sut.Process(input); Assert.AreEqual(expected, sut.State)` |
| Mock verification (Moq) | `mock.Verify(x => x.Save(It.IsAny<Data>()), Times.Once)` |
| Mock verification (NSubstitute) | `sub.Received(1).Save(Arg.Any<Data>())` |
| Event assertion | `Assert.IsTrue(eventFired)` after triggering |

**Moq/NSubstitute note:** `.Verify()` and `.Received()` are explicit assertion calls. Classify as State/Side-effect.

### Structural/Deep
| Pattern | Example |
|---------|---------|
| Nested property access | `Assert.AreEqual(expected, result.Inner.Child.Value)` |
| JSON comparison | `Assert.AreEqual(expectedJson, JsonSerializer.Serialize(obj))` |
| Object graph equality | `expected.Should().BeEquivalentTo(actual)` (FluentAssertions) |
| Snapshot testing | `Verify(result)` (Verify library) |

### Implicit Assertions
| Pattern | Example | Notes |
|---------|---------|-------|
| Moq strict mock | `new Mock<IService>(MockBehavior.Strict)` | Throws on unexpected calls — implicit side-effect assertion |
| `using` / `Dispose` on mock | Mock.Verify runs in Dispose | Deferred assertion |
| No explicit assert in integration test | Test relies on exception for failure | Flag as implicit-only |

## Calibration Rules (.NET-Specific)

1. **MSTest, xUnit, NUnit, and TUnit use different method names for the same concept.** `Assert.AreEqual` (MSTest) = `Assert.Equal` (xUnit) = `Assert.That(x, Is.EqualTo(y))` (NUnit) = `await Assert.That(x).IsEqualTo(y)` (TUnit). Classify all correctly into the same category.

2. **FluentAssertions chains are single assertions.** `result.Should().NotBeNull().And.HaveCount(3)` is two assertions (Null + Collection), not one.

3. **`[ExpectedException]` attribute (MSTest legacy) is equivalent to `Assert.ThrowsException`.** Classify as Error/Exception. But note: `[ExpectedException]` is less precise because it catches the exception anywhere in the test method, not just from the specific call.

4. **`AutoFixture` / `Bogus` setup is not an assertion.** Test data generation is arrangement, not verification.

5. **`Assert.Inconclusive()` is not an assertion.** It marks a test as skipped/incomplete.

6. **OneOf/Result type pattern matching counts as Type/Pattern.** `Assert.IsTrue(result.IsT0)` is a type check; `result.AsT0` followed by value assertions is a guard + deeper check.

## Multi-Category Examples

| Code | Categories |
|------|-----------|
| `Assert.ThrowsException<ArgumentNullException>(() => ...)` | Error/Exception (+ implicitly Negative) |
| `Assert.AreNotEqual(0, list.Count)` | Inequality + Collection + Negative |
| `StringAssert.Contains(error.Message, "not found")` | String |
| `mock.Verify(x => x.Save(data), Times.Once)` | State/Side-effect |
| `Assert.IsInstanceOfType(result, typeof(SuccessResult))` | Type/Pattern |
| `Assert.That(items, Has.Count.EqualTo(3).And.All.Matches<Item>(x => x.IsValid))` | Collection + Boolean |
| `Assert.DoesNotThrow(() => sut.Validate())` | Negative + Error/Exception |
