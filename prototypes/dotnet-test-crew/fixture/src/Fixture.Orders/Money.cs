namespace Fixture.Orders;

/// <summary>
/// A validating value type. Two constructor guards, currency normalization, and
/// an Add path that rejects currency mismatches — none of it tested by the
/// bundled MoneyTests except the happy constructor + ToString.
/// </summary>
public readonly struct Money
{
    public decimal Amount { get; }
    public string Currency { get; }

    public Money(decimal amount, string currency)
    {
        if (amount < 0)
            throw new System.ArgumentOutOfRangeException(nameof(amount), "amount must be non-negative");
        if (string.IsNullOrWhiteSpace(currency))
            throw new System.ArgumentException("currency required", nameof(currency));
        Amount = amount;
        Currency = currency.ToUpperInvariant();
    }

    /// <summary>Happy path adds; mismatched currency throws (error-path gap).</summary>
    public Money Add(Money other)
    {
        if (other.Currency != Currency)
            throw new System.InvalidOperationException($"currency mismatch: {Currency} vs {other.Currency}");
        return new Money(Amount + other.Amount, Currency);
    }

    public override string ToString() => $"{Amount:0.00} {Currency}";
}
