package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class OrderJsonTest {
    @Test
    void roundTripsCents() throws Exception {
        assertEquals(1250, OrderJson.cents(OrderJson.write("a1", 1250)));
    }
}
