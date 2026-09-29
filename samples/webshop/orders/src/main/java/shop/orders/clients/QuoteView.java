package shop.orders.clients;

import shop.common.Money;

public record QuoteView(Money subtotal, Money discount, String rule, Money tax, Money total) {
}
