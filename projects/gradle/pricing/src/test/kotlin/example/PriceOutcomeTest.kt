package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class PriceOutcomeTest {
    @Test fun formatsReadyAndInvalidPrices() {
        assertEquals("EUR:120", 100.toPriceOutcome().label())
        assertEquals("invalid:-1", (-1).toPriceOutcome().label())
    }
}
