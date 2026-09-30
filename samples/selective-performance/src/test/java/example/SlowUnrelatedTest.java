package example;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;

class SlowUnrelatedTest {
    @Test void expensiveIndependentSetup() throws InterruptedException {
        // A controlled stand-in for unrelated application/container startup, not real I/O.
        Thread.sleep(Long.getLong("sample.delay.ms", 6000L));
        assertEquals("AVAILABLE", "available".toUpperCase(java.util.Locale.ROOT));
    }
}
