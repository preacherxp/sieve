package example

sealed interface Discount {
    data object None : Discount

    data class Percent(val percent: Int) : Discount

    data class Fixed(val amount: Cents) : Discount
}

fun Discount.applyTo(total: Cents): Cents = when (this) {
    Discount.None -> total
    is Discount.Percent -> Cents(total.value * (100 - percent) / 100)
    is Discount.Fixed -> Cents(maxOf(0, total.value - amount.value))
}
