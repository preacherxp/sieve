package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class MultiplicationTest {
    @Test
    void multiplies() {
        assertEquals(6, new Calculator().multiply(2, 3));
    }
}
