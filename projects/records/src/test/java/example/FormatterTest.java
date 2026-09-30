package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class FormatterTest {
    @Test
    void formats() {
        assertEquals("#7", new Formatter().format(7));
    }
}
