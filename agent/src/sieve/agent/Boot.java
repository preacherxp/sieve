package sieve.agent;

import java.lang.instrument.Instrumentation;

/**
 * The agent's entry point, compiled for Java 8: `JDK_JAVA_OPTIONS` reaches every `java` launch,
 * and an entry point that an old JVM cannot load would abort it.
 */
public final class Boot {
    private Boot() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        String version = System.getProperty("java.specification.version", "0");
        int feature;
        try {
            feature = Integer.parseInt(version.startsWith("1.") ? version.substring(2) : version);
        } catch (NumberFormatException error) {
            feature = 0;
        }
        if (feature < 24) {
            if (arguments != null && !arguments.isEmpty() && !isBuildTool()) {
                System.err.println("sieve: test records need a Java 24+ test JVM, not " + version + "; every test runs");
            }
            return;
        }
        try {
            Class.forName("sieve.agent.Agent")
                    .getMethod("premain", String.class, Instrumentation.class)
                    .invoke(null, arguments, instrumentation);
        } catch (Throwable error) {
            System.err.println("sieve: cannot start the agent, every test runs: " + error);
        }
    }

    static boolean isBuildTool() {
        String command = System.getProperty("sun.java.command", "");
        return command.startsWith("org.codehaus.plexus.classworlds.launcher.Launcher") || command.startsWith("org.mvndaemon.");
    }
}
