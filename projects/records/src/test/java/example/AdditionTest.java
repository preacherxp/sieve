package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class AdditionTest {
    @Test
    void adds() {
        assertEquals(5, new Calculator().add(2, 3));
    }
}
