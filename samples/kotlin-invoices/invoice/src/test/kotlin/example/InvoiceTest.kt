package example

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class InvoiceTest {
    private val invoice = Invoice(listOf(Line("tea", Cents(250), 2), Line("pot", Cents(1999))))

    @Test
    fun subtotalAddsLines() {
        assertEquals(Cents(2499), invoice.subtotal())
    }

    @Test
    fun dueWithoutDiscountIsTheSubtotal() {
        assertEquals(invoice.subtotal(), invoice.due())
    }
}
