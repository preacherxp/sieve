package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class ReceiptTest {
    @Test
    void namesAndPricesALine() {
        assertEquals("tea-pot 9.99", Receipt.line("Tea Pot", 999));
    }
}
