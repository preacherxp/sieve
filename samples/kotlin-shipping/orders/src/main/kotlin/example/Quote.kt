package example

object Quote {
    fun price(parcel: Parcel): Int =
        (Rates.BASE_FEE + Rates.PER_KILO * parcel.kilos) * Zone.of(parcel.country).multiplier
}
