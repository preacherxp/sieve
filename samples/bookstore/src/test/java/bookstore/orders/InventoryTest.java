package bookstore.orders;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class InventoryTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void reservesOnlyAvailableStock() {
        Inventory inventory = new Inventory();
        inventory.restock("isbn", 2);
        assertTrue(inventory.reserve("isbn", 2));
        assertFalse(inventory.reserve("isbn", 1));
        assertEquals(0, inventory.available("isbn"));
    }
}
