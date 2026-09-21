package example;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class PriceQuoteTest {
    @Test void recordCarriesCalculatedPrice() {
        assertEquals(new PriceQuote(100, 120), PriceQuote.priced(100));
        assertEquals(120, PriceQuote.priced(100).total());
    }

    @Test void rejectsInvalidPrice() {
        assertThrows(IllegalArgumentException.class, () -> PriceQuote.priced(-1));
    }
}
