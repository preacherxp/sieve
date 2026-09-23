package bookstore.orders;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import bookstore.catalog.Catalog;
import bookstore.pricing.PriceCalculator;
import java.math.BigDecimal;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class OrderServiceIT {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void placesPricedOrderAndReservesStock() {
        Catalog catalog = new Catalog();
        catalog.add(TestEnvironment.EMMA);
        Inventory inventory = new Inventory();
        inventory.restock(TestEnvironment.EMMA.isbn(), 3);
        OrderService service = new OrderService(catalog, inventory, PriceCalculator.standard());
        Order order = service.place("ada", TestEnvironment.EMMA.isbn(), 2, false);
        assertEquals(new BigDecimal("17.12"), order.total());
        assertEquals(1, inventory.available(TestEnvironment.EMMA.isbn()));
        assertThrows(IllegalStateException.class, () -> service.place("ada", TestEnvironment.EMMA.isbn(), 2, false));
    }
}
