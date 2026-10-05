package example

/** Inline: callers, in this module or another, carry a copy of this body in their bytecode. */
inline fun <T> Iterable<T>.sumOfCents(amount: (T) -> Cents): Cents {
    var total = Cents(0)
    for (item in this) {
        total += amount(item)
    }
    return total
}
