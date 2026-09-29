package bookstore.pricing;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import java.math.BigDecimal;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class BulkDiscountTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void tenOrMoreCopiesSaveFive() {
        assertEquals(new BigDecimal("95.00"), new BulkDiscount().apply(new BigDecimal("100.00"), 10, false));
        assertEquals(new BigDecimal("90.00"), new BulkDiscount().apply(new BigDecimal("90.00"), 9, false));
    }
}
