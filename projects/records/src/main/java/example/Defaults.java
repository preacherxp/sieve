package example;

import java.util.List;

/** Read only through a static field, whose initializer runs in whichever test comes first. */
public final class Defaults {
    public static final List<String> NAMES = List.of("a", "b");

    private Defaults() {}
}
