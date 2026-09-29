package shop.pricing;

import shop.common.Money;

public record PriceRequest(String sku, String category, Money unitPrice, int quantity) {
    public Money subtotal() {
        return unitPrice.times(quantity);
    }
}
