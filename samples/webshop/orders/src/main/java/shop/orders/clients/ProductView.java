package shop.orders.clients;

import shop.common.Money;

/** The part of the catalog's product that orders needs. */
public record ProductView(String sku, String name, String category, Money price) {
}
