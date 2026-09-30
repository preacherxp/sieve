package example;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class PriceTest {
    @Test void totalIncludesTax() {
        assertEquals(12, new Price().total(10, 2));
    }
}
