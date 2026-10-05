package example

data class Line(val name: String, val price: Cents, val quantity: Int = 1)

class Invoice(private val lines: List<Line>, private val discount: Discount = Discount.None) {
    fun subtotal(): Cents = lines.sumOfCents { it.price * it.quantity }

    fun due(): Cents = discount.applyTo(subtotal())
}
