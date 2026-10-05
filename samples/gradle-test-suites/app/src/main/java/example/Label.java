package example;

public final class Label {
    public static String of(String name) {
        return "label:" + Slug.format(name);
    }
}
