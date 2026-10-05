package example;

import java.util.Locale;

public final class Slug {
    public static String format(String value) {
        return value.toUpperCase(Locale.ROOT).replace(' ', '-');
    }
}
