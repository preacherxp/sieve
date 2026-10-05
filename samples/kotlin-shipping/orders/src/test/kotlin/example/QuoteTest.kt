package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class QuoteTest {
    @Test
    fun domestic() {
        assertEquals(619, Quote.price(Parcel(500, "PL")))
    }

    @Test
    fun europeDoublesThePrice() {
        assertEquals(1718, Quote.price(Parcel(2500, "DE")))
    }
}
