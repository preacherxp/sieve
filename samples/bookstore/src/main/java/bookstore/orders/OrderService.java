package bookstore.orders;

import bookstore.catalog.Book;
import bookstore.catalog.Catalog;
import bookstore.pricing.PriceCalculator;

public class OrderService {
    private final Catalog catalog;
    private final Inventory inventory;
    private final PriceCalculator prices;

    public OrderService(Catalog catalog, Inventory inventory, PriceCalculator prices) {
        this.catalog = catalog;
        this.inventory = inventory;
        this.prices = prices;
    }

    public Order place(String customer, String isbn, int quantity, boolean member) {
        Book book = catalog.find(isbn).orElseThrow(() -> new IllegalArgumentException("Unknown book " + isbn));
        if (!inventory.reserve(isbn, quantity)) {
            throw new IllegalStateException("Out of stock: " + isbn);
        }
        return new Order(customer, book, quantity, prices.total(book, quantity, member));
    }
}
