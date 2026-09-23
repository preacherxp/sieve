package bookstore.pricing;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import java.math.BigDecimal;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class MemberDiscountTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void membersPayNinetyPercent() {
        assertEquals(new BigDecimal("9.0000"), new MemberDiscount().apply(new BigDecimal("10.00"), 1, true));
        assertEquals(new BigDecimal("10.00"), new MemberDiscount().apply(new BigDecimal("10.00"), 1, false));
    }
}
