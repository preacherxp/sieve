package example

data class Parcel(val grams: Int, val country: String)

/** Billable weight: whole kilos, rounded up. */
val Parcel.kilos: Int
    get() = (grams + 999) / 1000
