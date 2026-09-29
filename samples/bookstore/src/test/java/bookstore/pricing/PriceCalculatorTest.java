package bookstore.pricing;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import java.math.BigDecimal;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class PriceCalculatorTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void appliesRulesThenTax() {
        assertEquals(new BigDecimal("90.95"), PriceCalculator.standard().total(TestEnvironment.DUNE, 10, true));
    }
}
