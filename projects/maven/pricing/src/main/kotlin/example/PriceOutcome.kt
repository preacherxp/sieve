package example

sealed interface PriceOutcome {
    data class Ready(val total: Int) : PriceOutcome
    data class Invalid(val net: Int) : PriceOutcome
}

fun Int.toPriceOutcome(): PriceOutcome =
    if (this < 0) PriceOutcome.Invalid(this) else PriceOutcome.Ready(PriceCalculator().total(this))

fun PriceOutcome.label(): String = when (this) {
    is PriceOutcome.Ready -> "EUR:$total"
    is PriceOutcome.Invalid -> "invalid:$net"
}
