package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class FormatTest {
    @Test
    fun padsMinorUnits() {
        assertEquals("12.05", Cents(1205).format())
    }
}
