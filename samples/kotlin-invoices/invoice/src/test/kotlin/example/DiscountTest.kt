package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class DiscountTest {
    @Test
    fun percentOff() {
        assertEquals(Cents(1800), Discount.Percent(10).applyTo(Cents(2000)))
    }

    @Test
    fun fixedNeverGoesNegative() {
        assertEquals(Cents(0), Discount.Fixed(Cents(500)).applyTo(Cents(300)))
    }
}
