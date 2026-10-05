package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class ParcelTest {
    @Test
    fun roundsUpToWholeKilos() {
        assertEquals(2, Parcel(1001, "PL").kilos)
    }
}
