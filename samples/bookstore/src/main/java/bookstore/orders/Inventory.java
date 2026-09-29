package bookstore.orders;

import java.util.HashMap;
import java.util.Map;

public class Inventory {
    private final Map<String, Integer> stock = new HashMap<>();

    public void restock(String isbn, int quantity) {
        stock.merge(isbn, quantity, Integer::sum);
    }

    public boolean reserve(String isbn, int quantity) {
        int available = stock.getOrDefault(isbn, 0);
        if (available < quantity) {
            return false;
        }
        stock.put(isbn, available - quantity);
        return true;
    }

    public int available(String isbn) {
        return stock.getOrDefault(isbn, 0);
    }
}
