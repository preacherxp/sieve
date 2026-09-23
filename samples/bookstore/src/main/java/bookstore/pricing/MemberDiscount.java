package bookstore.pricing;

import java.math.BigDecimal;

public class MemberDiscount implements PriceRule {
    private static final BigDecimal RATE = new BigDecimal("0.90");

    @Override
    public BigDecimal apply(BigDecimal total, int quantity, boolean member) {
        return member ? total.multiply(RATE) : total;
    }
}
