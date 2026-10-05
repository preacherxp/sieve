package example;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class SlugTest {
    @Test
    void stripsAccentsAndPunctuation() {
        assertEquals("creme-brulee", Slug.of("Crème Brûlée!"));
    }
}
