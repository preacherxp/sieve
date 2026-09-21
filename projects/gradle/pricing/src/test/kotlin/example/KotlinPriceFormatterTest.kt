package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class KotlinPriceFormatterTest {
    @Test fun formatsJavaCalculatedPrice() {
        assertEquals("EUR:120", KotlinPriceFormatter().format(100))
    }
}
