package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;

@AppTest
class ComposedGreetingTest {
    @Autowired
    GreetingService service;

    @Test
    void greetsWithTheComposedConfiguration() {
        assertEquals("Hello, Ada", service.greeting("Ada"));
    }
}
