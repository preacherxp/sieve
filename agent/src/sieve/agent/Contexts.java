package sieve.agent;

/** Public entry points for the Spring hooks, which live in their own package. */
public final class Contexts {
    private Contexts() {}

    public static void starting(Object context) {
        State.contextStarting(context);
    }

    public static void started(Object context) {
        State.contextStarted(context);
    }

    public static void used(String testClass, Object context) {
        State.usesContext(testClass, context);
    }

    public static void bootStarting() {
        State.bootStarting();
    }

    public static void bootFinished() {
        State.bootFinished();
    }
}
