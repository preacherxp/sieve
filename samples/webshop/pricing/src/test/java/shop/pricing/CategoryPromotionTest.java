package shop.pricing;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import shop.common.Money;

class CategoryPromotionTest {
    private final CategoryPromotion rule = new CategoryPromotion();

    @Test
    void discountsBooks() {
        assertThat(rule.discount(new PriceRequest("BOOK-1", "books", Money.of(3999), 1))).isEqualTo(Money.of(400));
    }

    @Test
    void ignoresHardware() {
        assertThat(rule.discount(new PriceRequest("KBD-1", "hardware", Money.of(12900), 1))).isEqualTo(Money.zero());
    }
}
