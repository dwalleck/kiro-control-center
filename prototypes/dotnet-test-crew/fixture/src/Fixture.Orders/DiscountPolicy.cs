namespace Fixture.Orders;

/// <summary>
/// Tiered discount logic that composes with <see cref="Money"/>. Four rate tiers
/// plus a negative-quantity error path, and an Apply method that crosses into
/// Money's own branches. Entirely untested by the bundled tests.
/// </summary>
public static class DiscountPolicy
{
    /// <summary>0–9 → 0%, 10–49 → 5%, 50–99 → 10%, 100+ → 15%; negative throws.</summary>
    public static decimal RateForQuantity(int quantity)
    {
        if (quantity < 0)
            throw new System.ArgumentOutOfRangeException(nameof(quantity), "quantity must be non-negative");
        if (quantity < 10) return 0.00m;
        if (quantity < 50) return 0.05m;
        if (quantity < 100) return 0.10m;
        return 0.15m;
    }

    /// <summary>Cross-class composition: prices a quantity then applies the tier rate.</summary>
    public static Money Apply(Money unitPrice, int quantity)
    {
        var gross = new Money(unitPrice.Amount * quantity, unitPrice.Currency);
        var rate = RateForQuantity(quantity);
        var discounted = gross.Amount * (1 - rate);
        return new Money(discounted, unitPrice.Currency);
    }
}
