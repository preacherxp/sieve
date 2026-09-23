package bookstore.report;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import bookstore.orders.Order;
import java.math.BigDecimal;
import java.util.List;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class SalesReportTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void sumsRevenuePerAuthor() {
        List<Order> orders = List.of(
                new Order("a", TestEnvironment.DUNE, 1, new BigDecimal("10.70")),
                new Order("b", TestEnvironment.DUNE, 1, new BigDecimal("10.70")),
                new Order("c", TestEnvironment.EMMA, 1, new BigDecimal("8.56")));
        assertEquals(new BigDecimal("21.40"), new SalesReport().revenueByAuthor(orders).get("Frank Herbert"));
    }
}
