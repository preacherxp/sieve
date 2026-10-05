package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class ZoneTest {
    @Test
    fun neighboursAreEurope() {
        assertEquals(Zone.EUROPE, Zone.of("FR"))
    }

    @Test
    fun unknownCountriesAreWorld() {
        assertEquals(Zone.WORLD, Zone.of("JP"))
    }
}
