package shop.pricing;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import shop.common.Money;

class TaxPolicyTest {
    private final TaxPolicy policy = new TaxPolicy();

    @Test
    void booksUseReducedRate() {
        assertThat(policy.tax("books", Money.of(10000))).isEqualTo(Money.of(500));
    }

    @Test
    void otherGoodsUseStandardRate() {
        assertThat(policy.tax("hardware", Money.of(10000))).isEqualTo(Money.of(2000));
    }
}
