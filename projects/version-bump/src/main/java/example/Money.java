package example;

/** Plain arithmetic: no library. */
public final class Money {
    private Money() {}

    public static String format(long cents) {
        return String.format("%d.%02d", cents / 100, cents % 100);
    }
}
