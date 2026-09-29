package shop.common;

import java.util.Objects;

/** An amount in minor units, so prices never pick up floating point drift. */
public record Money(long cents, String currency) {
    public static final String DEFAULT_CURRENCY = "EUR";

    public Money {
        Objects.requireNonNull(currency, "currency");
        if (cents < 0) {
            throw new IllegalArgumentException("Negative amount: " + cents);
        }
    }

    public static Money of(long cents) {
        return new Money(cents, DEFAULT_CURRENCY);
    }

    public static Money zero() {
        return of(0);
    }

    public Money plus(Money other) {
        requireSameCurrency(other);
        return new Money(Math.addExact(cents, other.cents), currency);
    }

    public Money minus(Money other) {
        requireSameCurrency(other);
        return new Money(cents - other.cents, currency);
    }

    public Money times(int quantity) {
        return new Money(Math.multiplyExact(cents, quantity), currency);
    }

    /** Rounds half up to the nearest cent. */
    public Money percent(int percent) {
        return new Money((cents * percent + 50) / 100, currency);
    }

    public boolean greaterThan(Money other) {
        requireSameCurrency(other);
        return cents > other.cents;
    }

    public String format() {
        return String.format("%s %d.%02d", currency, cents / 100, cents % 100);
    }

    private void requireSameCurrency(Money other) {
        if (!currency.equals(other.currency)) {
            throw new IllegalArgumentException(currency + " vs " + other.currency);
        }
    }
}
