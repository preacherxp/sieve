package bookstore.pricing;

import java.math.BigDecimal;

public class BulkDiscount implements PriceRule {
    @Override
    public BigDecimal apply(BigDecimal total, int quantity, boolean member) {
        return quantity >= 10 ? total.subtract(new BigDecimal("5.00")) : total;
    }
}
