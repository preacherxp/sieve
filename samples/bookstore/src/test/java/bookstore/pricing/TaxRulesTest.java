package bookstore.pricing;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import java.math.BigDecimal;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class TaxRulesTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void addsBookTax() {
        assertEquals(new BigDecimal("10.70"), TaxRules.books().withTax(new BigDecimal("10.00")));
    }
}
