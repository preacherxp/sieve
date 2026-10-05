package example;

import org.apache.commons.lang3.StringUtils;

/** URL slugs, through commons-lang3. */
public final class Slug {
    private Slug() {}

    public static String of(String title) {
        return StringUtils.stripAccents(title).toLowerCase().replaceAll("[^a-z0-9]+", "-").replaceAll("(^-|-$)", "");
    }
}
