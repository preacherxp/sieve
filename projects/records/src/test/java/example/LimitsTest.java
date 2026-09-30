package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

class LimitsTest {
    @Test
    void readsAFixtureByPath() throws IOException {
        assertEquals("max=10", Files.readString(Path.of("src/test/resources/limits.txt")).trim());
    }
}
