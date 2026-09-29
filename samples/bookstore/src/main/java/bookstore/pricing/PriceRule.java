package bookstore.pricing;

import java.math.BigDecimal;

/** Adjusts a line total; implementations are applied in order. */
public interface PriceRule {
    BigDecimal apply(BigDecimal total, int quantity, boolean member);
}
