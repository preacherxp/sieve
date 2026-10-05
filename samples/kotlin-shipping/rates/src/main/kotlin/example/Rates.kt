package example

/** Fees in cents. `const` values are copied into every caller's bytecode at compile time. */
object Rates {
    const val BASE_FEE = 499
    const val PER_KILO = 120
}
