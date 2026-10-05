package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class SumsTest {
    @Test
    fun addsEveryItem() {
        assertEquals(Cents(600), listOf(1, 2, 3).sumOfCents { Cents(it * 100L) })
    }
}
