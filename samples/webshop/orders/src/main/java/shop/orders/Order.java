package shop.orders;

import shop.common.Money;

public record Order(String id, String sku, int quantity, String email, Money total, String pricingRule, OrderStatus status) {
}
