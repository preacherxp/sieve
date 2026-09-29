package shop.pricing;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import shop.common.Money;

class BulkDiscountTest {
    private final BulkDiscount rule = new BulkDiscount();

    @Test
    void appliesFromTenItems() {
        assertThat(rule.discount(new PriceRequest("MUG-1", "merch", Money.of(1000), 10))).isEqualTo(Money.of(500));
    }

    @Test
    void ignoresSmallOrders() {
        assertThat(rule.discount(new PriceRequest("MUG-1", "merch", Money.of(1000), 9))).isEqualTo(Money.zero());
    }
}
