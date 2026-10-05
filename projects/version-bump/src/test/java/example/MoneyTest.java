package example;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;

class MoneyTest {
    @Test
    void padsCents() {
        assertThat(Money.format(1205)).isEqualTo("12.05");
    }
}
