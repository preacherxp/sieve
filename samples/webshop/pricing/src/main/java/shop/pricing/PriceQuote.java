package shop.pricing;

import shop.common.Money;

public record PriceQuote(Money subtotal, Money discount, String rule, Money tax, Money total) {
}
