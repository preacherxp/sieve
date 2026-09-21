package example

class KotlinPriceFormatter {
    fun format(net: Int): String = "EUR:" + PriceCalculator().total(net)
}
