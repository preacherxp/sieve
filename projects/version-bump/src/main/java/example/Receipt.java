package example;

/** Reaches commons-lang3 only through {@link Slug}. */
public final class Receipt {
    private Receipt() {}

    public static String line(String title, long cents) {
        return Slug.of(title) + " " + Money.format(cents);
    }
}
