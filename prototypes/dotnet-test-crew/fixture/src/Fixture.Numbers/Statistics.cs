namespace Fixture.Numbers;

/// <summary>
/// A second class in the Numbers module so the module is multi-file. Every
/// method has an empty-input error path plus value branches — all untested by
/// the bundled tests.
/// </summary>
public static class Statistics
{
    /// <summary>Throws on null/empty (error-path gap); otherwise averages.</summary>
    public static double Mean(IReadOnlyCollection<int> values)
    {
        if (values is null || values.Count == 0)
            throw new System.ArgumentException("values must be non-empty", nameof(values));
        long sum = 0;
        foreach (var v in values) sum += v;
        return (double)sum / values.Count;
    }

    /// <summary>Throws on null/empty (error-path gap); otherwise the maximum.</summary>
    public static int Max(IReadOnlyCollection<int> values)
    {
        if (values is null || values.Count == 0)
            throw new System.ArgumentException("values must be non-empty", nameof(values));
        var max = int.MinValue;
        foreach (var v in values)
            if (v > max) max = v;
        return max;
    }

    /// <summary>Three-way branch: up / down / flat.</summary>
    public static string Trend(int previous, int current)
    {
        if (current > previous) return "up";
        if (current < previous) return "down";
        return "flat";
    }
}
