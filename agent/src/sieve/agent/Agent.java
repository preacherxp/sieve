package sieve.agent;

import java.io.IOException;
import java.io.InputStream;
import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Properties;
import java.util.Set;
import sieve.probe.Probe;

/**
 * Starts the agent on a Java 17+ JVM, called by {@link Boot} for
 * {@code -javaagent:sieve-agent.jar=<options file>}, which {@code sieve} passes through
 * {@code JDK_JAVA_OPTIONS}. The options file is written by the {@code sieve} binary; without it
 * the agent does nothing.
 */
public final class Agent {
    private Agent() {}

    public static void premain(String arguments, Instrumentation instrumentation) {
        if (arguments == null || arguments.isEmpty()) {
            return;
        }
        // JDK_JAVA_OPTIONS also reaches the build tool's own JVM, which runs no tests.
        if (Boot.isBuildTool()) {
            return;
        }
        Properties options = new Properties();
        try (InputStream in = Files.newInputStream(Path.of(arguments))) {
            options.load(in);
        } catch (IOException error) {
            System.err.println("sieve: cannot read agent options " + arguments + ": " + error);
            return;
        }
        String workspace = options.getProperty("workspace");
        String sieve = options.getProperty("sieve");
        if (workspace == null || sieve == null) {
            System.err.println("sieve: agent options lack workspace or sieve");
            return;
        }
        List<Path> outputs = new ArrayList<>();
        for (String dir : options.getProperty("outputs", "").split(java.io.File.pathSeparator)) {
            if (!dir.isEmpty()) {
                outputs.add(Path.of(dir).toAbsolutePath().normalize());
            }
        }
        // File reads count when they go through the JDK's file APIs, which are already loaded.
        List<Class<?>> jdk = new ArrayList<>(List.of(java.io.File.class, java.io.FileInputStream.class, java.io.RandomAccessFile.class));
        for (Class<?> c = FileSystems.getDefault().provider().getClass();
                c != null && c != java.nio.file.spi.FileSystemProvider.class;
                c = c.getSuperclass()) {
            jdk.add(c);
        }
        Set<String> names = new HashSet<>();
        for (Class<?> c : jdk) {
            names.add(c.getName().replace('.', '/'));
        }
        ClassFileTransformer transformer = new Transformer(outputs, Path.of(workspace), names);
        State.configure(Path.of(workspace), sieve, options.getProperty("mode", "select"), options.getProperty("base"),
                options.getProperty("context", ""), options.getProperty("session", ""), options.getProperty("record_env", ""),
                options.getProperty("ignore_properties", ""),
                Boolean.parseBoolean(options.getProperty("portable", "false")));
        instrumentation.addTransformer(transformer, true);
        try {
            instrumentation.retransformClasses(jdk.toArray(Class<?>[]::new));
        } catch (Throwable error) {
            State.error("retransform: " + error);
        }
        Probe.roots(Path.of(workspace));
    }
}
