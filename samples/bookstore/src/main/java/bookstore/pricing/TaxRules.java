package bookstore.pricing;

import java.math.BigDecimal;
import java.math.RoundingMode;

public class TaxRules {
    private final BigDecimal rate;

    public TaxRules(BigDecimal rate) {
        this.rate = rate;
    }

    public static TaxRules books() {
        return new TaxRules(new BigDecimal("0.07"));
    }

    public BigDecimal withTax(BigDecimal net) {
        return net.add(net.multiply(rate)).setScale(2, RoundingMode.HALF_UP);
    }
}
