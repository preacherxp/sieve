package example;

import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class DeliveryIT {
    @Test
    void usesSharedFixtureAndResource() throws Exception {
        try (var input = getClass().getResourceAsStream("/expected-label.txt")) {
            String expected = new String(input.readAllBytes(), StandardCharsets.UTF_8).strip();
            assertEquals(expected, Label.of(Examples.name()));
        }
    }
}
