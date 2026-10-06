package sieve.maven;

import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.concurrent.ConcurrentHashMap;
import javax.inject.Named;
import javax.inject.Singleton;
import org.apache.maven.execution.MojoExecutionEvent;
import org.apache.maven.execution.MojoExecutionListener;
import org.apache.maven.plugin.MojoExecution;

/**
 * Selects a module's test classes in the build that runs them. Right before Surefire or
 * Failsafe runs a module's tests, the module and everything it depends on are compiled:
 * {@code sieve classes} writes the module's excludes file, and the mojo reads it in place of
 * its own, which it extends. {@code sieve run} loads this extension with
 * {@code -Dmaven.ext.class.path} and passes {@code sieve.request} and {@code sieve.exe}. Compiled
 * for Java 8, as it runs in Maven's JVM. Whatever goes wrong, the module runs every test.
 */
@Named
@Singleton
public final class Select implements MojoExecutionListener {
    /** Excludes file per module directory; empty when every test runs. */
    private final Map<String, String> decided = new ConcurrentHashMap<>();

    @Override
    public void beforeMojoExecution(MojoExecutionEvent event) {
        try {
            Properties properties = event.getSession().getUserProperties();
            String request = properties.getProperty("sieve.request");
            String sieve = properties.getProperty("sieve.exe");
            MojoExecution execution = event.getExecution();
            String plugin = execution.getArtifactId();
            String goal = execution.getGoal();
            boolean tests = "maven-surefire-plugin".equals(plugin) && "test".equals(goal)
                    || "maven-failsafe-plugin".equals(plugin) && "integration-test".equals(goal);
            if (request == null || sieve == null || !tests) {
                return;
            }
            String dir = event.getProject().getBasedir().getAbsolutePath();
            String excludes = decided.computeIfAbsent(dir, d -> decide(sieve, request, d));
            if (excludes.isEmpty()) {
                return;
            }
            Object mojo = event.getMojo();
            Field field = field(mojo.getClass());
            field.setAccessible(true);
            File own = (File) field.get(mojo);
            field.set(mojo, own == null ? new File(excludes) : merged(own, new File(excludes)));
        } catch (Exception | LinkageError error) {
            System.err.println("[sieve] every test of this module runs: " + error);
        }
    }

    @Override
    public void afterMojoExecutionSuccess(MojoExecutionEvent event) {}

    @Override
    public void afterExecutionFailure(MojoExecutionEvent event) {}

    /** The excludes file {@code sieve} wrote for the module, or empty when every test runs. */
    private static String decide(String sieve, String request, String dir) {
        try {
            Process process = new ProcessBuilder(sieve, "classes", "--request", request, "--module", dir)
                    .redirectError(ProcessBuilder.Redirect.INHERIT)
                    .start();
            String path;
            try (BufferedReader out = new BufferedReader(
                    new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
                path = out.readLine();
            }
            return process.waitFor() == 0 && path != null ? path.trim() : "";
        } catch (IOException error) {
            System.err.println("[sieve] cannot select test classes: " + error);
            return "";
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            return "";
        }
    }

    private static Field field(Class<?> type) throws NoSuchFieldException {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                return c.getDeclaredField("excludesFile");
            } catch (NoSuchFieldException ignored) {
                // Declared further up.
            }
        }
        throw new NoSuchFieldException("excludesFile");
    }

    /** The POM's own excludes followed by the unselected test classes. */
    private static File merged(File own, File excludes) throws IOException {
        if (!own.isFile()) {
            // The mojo reports a missing file of its own configuration itself.
            return own;
        }
        List<String> lines = new ArrayList<>(Files.readAllLines(own.toPath(), StandardCharsets.UTF_8));
        lines.addAll(Files.readAllLines(excludes.toPath(), StandardCharsets.UTF_8));
        File file = new File(excludes.getPath() + ".merged");
        Files.write(file.toPath(), lines, StandardCharsets.UTF_8);
        return file;
    }
}
