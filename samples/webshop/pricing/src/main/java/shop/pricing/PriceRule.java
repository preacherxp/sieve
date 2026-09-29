package shop.pricing;

import shop.common.Money;

/** A discount candidate. The pricing service applies the largest one. */
public interface PriceRule {
    String name();

    Money discount(PriceRequest request);
}
