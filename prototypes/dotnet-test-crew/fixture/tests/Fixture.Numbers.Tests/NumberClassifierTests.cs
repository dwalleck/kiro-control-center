using Xunit;

namespace Fixture.Numbers.Tests;

// INTENTIONALLY PARTIAL. This file covers exactly one branch of one method so
// the fixture starts green while leaving a large, obvious coverage gap:
//   - Classify: negative / small / large branches are untested
//   - IsPrime:  every branch is untested
//   - Factorial: happy path AND the ArgumentOutOfRangeException path are untested
// The pipeline (and, in the loop variant, the validator) should detect and close
// these. Append new tests below — this file is the existing test project the
// implementer is expected to grow.
public class NumberClassifierTests
{
    [Fact]
    public void Classify_Zero_ReturnsZero()
    {
        var sut = new NumberClassifier();
        Assert.Equal("zero", sut.Classify(0));
    }
}
