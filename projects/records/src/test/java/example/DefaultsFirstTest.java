package example;

import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class DefaultsFirstTest {
    @Test
    void readsTheDefaults() {
        assertTrue(Defaults.NAMES.contains("a"));
    }
}
