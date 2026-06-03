using Fixture.Orders;

namespace Fixture.Orders.Tests;

// INTENTIONALLY PARTIAL — the existing test project for the Orders module.
// Covers only the happy constructor + ToString. Known gaps the pipeline must close:
//   - Money: negative-amount guard, blank-currency guard, currency normalization,
//            Add happy path, Add currency-mismatch throw
//   - DiscountPolicy: all four rate tiers, the negative-quantity throw, and Apply
// Append new tests below (append-only); add a DiscountPolicyTests.cs for that class.
public class MoneyTests
{
    [Fact]
    public void Constructor_NormalizesAndStores()
    {
        var money = new Money(9.5m, "usd");
        Assert.Equal(9.5m, money.Amount);
        Assert.Equal("USD", money.Currency);
        Assert.Equal("9.50 USD", money.ToString());
    }
}
