package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class GreeterTest {
    @Test
    void greetsThroughTheSupertype() {
        Greeter greeter = new LoudGreeter();
        assertEquals("hi ada", greeter.greet("ada"));
    }
}
