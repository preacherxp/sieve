package example

enum class Zone(val multiplier: Int) {
    DOMESTIC(1),
    EUROPE(2),
    WORLD(4);

    companion object {
        fun of(country: String): Zone = when (country) {
            "PL" -> DOMESTIC
            "DE", "FR", "CZ" -> EUROPE
            else -> WORLD
        }
    }
}
