package bookstore.report;

import bookstore.orders.Order;
import java.math.BigDecimal;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

public class SalesReport {
    public Map<String, BigDecimal> revenueByAuthor(List<Order> orders) {
        Map<String, BigDecimal> revenue = new TreeMap<>();
        for (Order order : orders) {
            revenue.merge(order.book().author(), order.total(), BigDecimal::add);
        }
        return revenue;
    }
}
