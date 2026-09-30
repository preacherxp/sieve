package sieve.agent;

/** Public entry points for the Spring hooks, which live in their own package. */
public final class Contexts {
    private Contexts() {}

    public static void starting(Object context) {
        if (State.enabled()) {
            State.contextStarting(context);
        }
    }

    public static void started(Object context) {
        if (State.enabled()) {
            State.contextStarted(context);
        }
    }

    public static void used(String testClass, Object context) {
        if (State.enabled()) {
            State.usesContext(testClass, context);
        }
    }

    public static void bootStarting() {
        if (State.enabled()) {
            State.bootStarting();
        }
    }

    public static void bootFinished() {
        if (State.enabled()) {
            State.bootFinished();
        }
    }

    /**
     * Test property files, which the context loader reads before the context starts, count as
     * read while it starts. Read by reflection: the accessor changed across Spring versions.
     */
    public static void propertySources(Object config) {
        if (!State.enabled()) {
            return;
        }
        try {
            String[] locations = (String[]) config.getClass().getMethod("getPropertySourceLocations").invoke(config);
            ClassLoader loader = Thread.currentThread().getContextClassLoader();
            for (String location : locations) {
                String path = location.replaceFirst("^classpath\\*?:/?", "");
                if (path.startsWith("file:")) {
                    sieve.probe.Probe.file(path.substring("file:".length()));
                    continue;
                }
                java.net.URL url = loader.getResource(path);
                if (url != null && "file".equals(url.getProtocol())) {
                    sieve.probe.Probe.file(java.nio.file.Path.of(url.toURI()));
                }
            }
        } catch (ReflectiveOperationException | java.net.URISyntaxException | RuntimeException error) {
            State.error("property sources: " + error);
        }
    }
}
