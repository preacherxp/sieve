package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.context.TestPropertySource;

/** Shares its context, and so its property file, with the other PropertySource test. */
@SpringBootTest
@TestPropertySource(locations = "classpath:greeting-it.properties")
class PropertySourceSecondTest {
    @Autowired
    GreetingService service;

    @Test
    void greetsWithTheTestProperties() {
        assertEquals("Hello, Ada", service.greeting("Ada"));
    }
}
