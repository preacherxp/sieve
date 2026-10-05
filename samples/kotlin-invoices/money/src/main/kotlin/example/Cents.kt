package example

/** An amount in minor units. */
@JvmInline
value class Cents(val value: Long) {
    operator fun plus(other: Cents) = Cents(value + other.value)

    operator fun times(quantity: Int) = Cents(value * quantity)
}
