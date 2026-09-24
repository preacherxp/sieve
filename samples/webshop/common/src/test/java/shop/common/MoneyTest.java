package shop.common;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

import org.junit.jupiter.api.Test;

class MoneyTest {
    @Test
    void addsAndMultiplies() {
        assertThat(Money.of(250).times(3).plus(Money.of(5))).isEqualTo(Money.of(755));
    }

    @Test
    void percentRoundsHalfUp() {
        assertThat(Money.of(1999).percent(5)).isEqualTo(Money.of(100));
        assertThat(Money.of(1990).percent(5)).isEqualTo(Money.of(100));
        assertThat(Money.of(1989).percent(5)).isEqualTo(Money.of(99));
    }

    @Test
    void rejectsNegativeAndMixedCurrencies() {
        assertThatThrownBy(() -> Money.of(1).minus(Money.of(2))).isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> Money.of(1).plus(new Money(1, "USD"))).isInstanceOf(IllegalArgumentException.class);
    }

    @Test
    void formats() {
        assertThat(Money.of(123405).format()).isEqualTo("EUR 1234.05");
    }
}
