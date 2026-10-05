package example;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class StatusIT {
    @Test
    void independentStatus() {
        assertEquals("ready", Status.value());
    }
}
