package example;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class IndependentTest {
    @Test
    void arithmetic() {
        assertEquals(4, Integer.sum(2, 2));
    }
}
