namespace Fixture.Numbers;

/// <summary>
/// Deterministic test target for the crew prototype. Every method below has
/// obvious, enumerable branches so a coverage-gap reviewer has an unambiguous
/// signal. The bundled test project intentionally covers only ONE branch of
/// one method — everything else is a known gap.
/// </summary>
public class NumberClassifier
{
    /// <summary>Four mutually exclusive branches: negative / zero / small / large.</summary>
    public string Classify(int n)
    {
        if (n < 0) return "negative";
        if (n == 0) return "zero";
        if (n < 100) return "small";
        return "large";
    }

    /// <summary>Branchy by construction: guard, even-divisor early exit, trial division.</summary>
    public bool IsPrime(int n)
    {
        if (n < 2) return false;
        for (var i = 2; (long)i * i <= n; i++)
        {
            if (n % i == 0) return false;
        }
        return true;
    }

    /// <summary>Throws on invalid input — the error path is a deliberate gap.</summary>
    public int Factorial(int n)
    {
        if (n < 0) throw new System.ArgumentOutOfRangeException(nameof(n), "n must be non-negative");
        var result = 1;
        for (var i = 2; i <= n; i++) result = checked(result * i);
        return result;
    }
}
