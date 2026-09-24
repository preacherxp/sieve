package shop.catalog;

import shop.common.Money;

public record Product(String sku, String name, String category, Money price) {
}
