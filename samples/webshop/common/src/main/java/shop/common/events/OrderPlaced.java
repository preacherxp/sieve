package shop.common.events;

import shop.common.Money;

/** Published by orders on its event stream and consumed by notifications. */
public record OrderPlaced(String orderId, String sku, String productName, int quantity, Money total, String email) {
}
