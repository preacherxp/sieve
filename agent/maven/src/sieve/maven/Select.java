package sieve.maven;

import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.Arrays;
import java.util.Set;
import java.util.TreeMap;
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
    /** Repeated executions cannot share the report identity required by a receipt. */
    private final Set<String> decided = ConcurrentHashMap.newKeySet();

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
            String suite = "maven-surefire-plugin".equals(plugin) ? "unit" : "integration";
            Object mojo = event.getMojo();
            boolean repeated = !decided.add(dir + ":" + suite);
            boolean receipt = "true".equals(properties.getProperty("sieve.receipt"));
            String excludes = decide(sieve, request, dir, suite, event.getProject(), mojo, repeated, receipt);
            if (excludes.isEmpty()) {
                return;
            }
            Field field = field(mojo.getClass());
            field.setAccessible(true);
            field.set(mojo, merged(mojo, new File(excludes)));
        } catch (Exception | LinkageError error) {
            System.err.println("[sieve] every test of this module runs: " + error);
        }
    }

    @Override
    public void afterMojoExecutionSuccess(MojoExecutionEvent event) {}

    @Override
    public void afterExecutionFailure(MojoExecutionEvent event) {}

    /** The excludes file {@code sieve} wrote for the module, or empty when every test runs. */
    private static String decide(String sieve, String request, String dir, String suite,
            Object project, Object mojo, boolean repeated, boolean receipt) {
        try {
            List<String> command = new ArrayList<>(Arrays.asList(
                    sieve, "classes", "--request", request, "--module", dir, "--suite", suite));
            if (repeated) command.add("--repeat");
            if (receipt) try {
                command.addAll(Arrays.asList("--classpath", inputs(request, suite, project, mojo).getAbsolutePath()));
            } catch (Exception | LinkageError error) {
                System.err.println("[sieve] execution receipt unavailable for this suite: " + error);
            }
            Process process = new ProcessBuilder(command)
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

    /** Resolved dependency bytes and effective test settings, before any test executes. */
    private static File inputs(String request, String suite, Object project, Object mojo) throws Exception {
        List<String> paths = new ArrayList<>();
        for (Object path : (Iterable<?>) project.getClass().getMethod("getTestClasspathElements").invoke(project)) {
            paths.add(path.toString());
        }
        for (String name : Arrays.asList("getAdditionalClasspathElements", "getSuiteXmlFiles")) {
            Object values = invoke(mojo, name);
            if (values != null) for (Object value : (Object[]) values) paths.add(value.toString());
        }
        Object dependencies = value(mojo, "additionalClasspathDependencies");
        if (dependencies instanceof List && !((List<?>) dependencies).isEmpty()) {
            throw new IOException("additionalClasspathDependencies require a resolved receipt adapter");
        }
        Object jvm = invoke(mojo, "getJvm");
        String launcher = jvm == null ? null : jvm.toString();
        if (launcher == null || launcher.isEmpty()) {
            Object toolchain = invoke(mojo, "getToolchain");
            if (toolchain != null) {
                launcher = (String) toolchain.getClass().getMethod("findTool", String.class).invoke(toolchain, "java");
            }
        }
        File launcherFile = launcher == null || launcher.isEmpty()
                ? new File(System.getProperty("java.home"), "bin/java") : new File(launcher);
        paths.add(launcherFile.getCanonicalPath());
        paths.add(new File(launcherFile.getCanonicalFile().getParentFile().getParentFile(), "release").getPath());
        Object propertiesFile = invoke(mojo, "getSystemPropertiesFile");
        if (propertiesFile != null) paths.add(propertiesFile.toString());
        Map<String, String> settings = new TreeMap<>();
        for (String name : Arrays.asList("getArgLine", "getSystemProperties", "getSystemPropertyVariables",
                "getEnvironmentVariables", "getGroups", "getExcludedGroups", "getIncludes", "getExcludes",
                "getForkCount", "isReuseForks", "getParallel", "getWorkingDirectory")) {
            Object setting = invoke(mojo, name);
            if (setting instanceof Map) setting = new TreeMap<>((Map<?, ?>) setting);
            settings.put(name, String.valueOf(setting));
        }
        // The extra agent can change behavior even when its path and arguments stay the same.
        String argLine = settings.get("getArgLine");
        if (argLine.contains("@{") || argLine.contains("-javaagent:")
                && (argLine.contains("\"") || argLine.contains("'"))) {
            throw new IOException("late or quoted agent arguments cannot be verified for a receipt");
        }
        java.util.regex.Matcher agents = java.util.regex.Pattern.compile("-javaagent:([^\\s=]+)").matcher(argLine);
        while (agents.find()) {
            File agent = new File(agents.group(1));
            if (!agent.isAbsolute()) agent = new File((File) invoke(mojo, "getWorkingDirectory"), agents.group(1));
            if (!agent.isFile() || !agent.canRead()) throw new IOException("cannot read Java agent for receipt: " + agent);
            paths.add(agent.getCanonicalPath());
        }
        for (String path : paths) {
            if (path.endsWith(".jar") && !new File(path).isFile()) {
                throw new IOException("cannot read classpath jar for receipt: " + path);
            }
        }
        File scratch = new File(request).getParentFile();
        File runtime = new File((File) invoke(mojo, "getProjectBuildDirectory"), "sieve-receipt/" + suite + ".sha256");
        Files.createDirectories(runtime.getParentFile().toPath());
        byte[] fingerprint = java.security.MessageDigest.getInstance("SHA-256")
                .digest(settings.toString().getBytes(StandardCharsets.UTF_8));
        Files.write(runtime.toPath(), fingerprint);
        paths.add(runtime.getAbsolutePath());
        File listing = File.createTempFile("classpath-", ".txt", scratch);
        Files.write(listing.toPath(), paths, StandardCharsets.UTF_8);
        return listing;
    }

    private static Object invoke(Object object, String name) throws Exception {
        for (Class<?> c = object.getClass(); c != null; c = c.getSuperclass()) {
            try {
                Method method = c.getDeclaredMethod(name);
                method.setAccessible(true);
                return method.invoke(object);
            } catch (NoSuchMethodException ignored) {}
        }
        throw new NoSuchMethodException(name);
    }

    private static Object value(Object object, String name) throws Exception {
        for (Class<?> c = object.getClass(); c != null; c = c.getSuperclass()) {
            try {
                Field field = c.getDeclaredField(name);
                field.setAccessible(true);
                return field.get(object);
            } catch (NoSuchFieldException ignored) {}
        }
        throw new NoSuchFieldException(name);
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

    /** Preserve the mojo's effective native exclusions, including its conditional defaults. */
    private static File merged(Object mojo, File excludes) throws Exception {
        List<String> lines = new ArrayList<>();
        for (Object nativeExclude : (Iterable<?>) invoke(mojo, "getExcludedScanList")) {
            lines.add(nativeExclude.toString());
        }
        lines.addAll(Files.readAllLines(excludes.toPath(), StandardCharsets.UTF_8));
        File file = new File(excludes.getPath() + ".merged");
        Files.write(file.toPath(), lines, StandardCharsets.UTF_8);
        return file;
    }
}
