package example;

import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;

class PriceTest {
    @Test
    void readsTheClasspathFixture() throws IOException {
        try (InputStream in = PriceTest.class.getResourceAsStream("/prices.json")) {
            assertTrue(new String(in.readAllBytes(), StandardCharsets.UTF_8).contains("\"apple\": 3"));
        }
    }
}
