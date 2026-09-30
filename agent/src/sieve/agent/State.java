package sieve.agent;

import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.Set;
import java.util.TreeSet;
import java.util.TreeMap;
import java.util.HexFormat;
import java.util.concurrent.ConcurrentHashMap;
import sieve.probe.Bucket;
import sieve.probe.Probe;

/**
 * What this test JVM ran, per top-level test class, and the conversation with the {@code sieve}
 * binary: it decides which classes to drop and turns the raw evidence into test records.
 */
final class State {
    private static Path workspace;
    private static String sieve;
    private static String mode = "off";
    private static String base;
    private static String invocation = "";
    private static String session = "";
    private static String[] recordEnv = new String[0];
    private static String context;

    private static final Map<String, Integer> IDS = new ConcurrentHashMap<>();
    private static final List<String> METHODS = Collections.synchronizedList(new ArrayList<>());
    private static final List<String> ERRORS = Collections.synchronizedList(new ArrayList<>());

    private static final class Run {
        final Bucket own = new Bucket();
        final Set<Bucket> contexts = Collections.newSetFromMap(new IdentityHashMap<>());
        boolean failed;
        boolean spring;
    }

    private static final Map<String, Run> RUNS = new LinkedHashMap<>();
    /** Weak, so that contexts the cache evicts can be collected; contexts use identity equality. */
    private static final Map<Object, Bucket> CONTEXTS = new java.util.WeakHashMap<>();
    private static final java.util.concurrent.atomic.AtomicInteger INSTRUMENTED = new java.util.concurrent.atomic.AtomicInteger();
    private static final Set<String> DROPPED = new TreeSet<>();
    private static final Set<String> FLUSHED = new LinkedHashSet<>();
    private static String active;
    private static boolean parallel;
    private static Set<String> skip;

    private State() {}

    static synchronized void configure(Path workspace, String sieve, String mode, String base, String invocation, String session, String recordEnv) {
        State.workspace = workspace;
        State.sieve = sieve;
        State.mode = mode;
        State.base = base;
        State.invocation = invocation;
        State.session = session;
        State.recordEnv = recordEnv.split(",");
    }

    /** Freeze at discovery: Surefire applies its test properties after agent premain. */
    private static String context() {
        if (context != null) {
            return context;
        }
        Map<String, String> values = new TreeMap<>();
        values.put("invocation", invocation);
        for (String name : System.getProperties().stringPropertyNames()) {
            // Classpath and command point at fresh Surefire booter files; compressed-oops
            // placement varies with ASLR. Project output and the JDK are checked separately.
            if (!Set.of("java.class.path", "sun.java.command", "surefire.real.class.path", "surefire.test.class.path", "java.vm.compressedOopsMode").contains(name)) {
                values.put("property:" + name, System.getProperty(name));
            }
        }
        for (String name : recordEnv) {
            if (!name.isEmpty()) {
                String value = System.getenv(name);
                values.put("env:" + name, value == null ? "absent" : "present:" + value);
            }
        }
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            for (Map.Entry<String, String> entry : values.entrySet()) {
                for (String value : List.of(entry.getKey(), entry.getValue())) {
                    byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
                    digest.update(java.nio.ByteBuffer.allocate(4).putInt(bytes.length).array());
                    digest.update(bytes);
                }
            }
            return context = HexFormat.of().formatHex(digest.digest());
        } catch (NoSuchAlgorithmException error) {
            throw new IllegalStateException(error);
        }
    }

    static boolean enabled() {
        return workspace != null && !mode.equals("off");
    }

    static int method(String name) {
        return IDS.computeIfAbsent(name, key -> {
            synchronized (METHODS) {
                METHODS.add(key);
                return METHODS.size() - 1;
            }
        });
    }

    static void error(String message) {
        ERRORS.add(message);
    }

    static void instrumented() {
        INSTRUMENTED.incrementAndGet();
    }

    static synchronized void parallel() {
        parallel = true;
    }

    /** {@code a.B$C} and {@code a.B} both belong to the record of {@code a.B}. */
    static String top(String className) {
        int nested = className.indexOf('$');
        return nested < 0 ? className : className.substring(0, nested);
    }

    private static String jdk() {
        return System.getProperty("java.vm.vendor", "") + " " + System.getProperty("java.vm.version", "");
    }

    private static boolean explicit() {
        for (String property : new String[] {"test", "it.test"}) {
            String value = System.getProperty(property);
            if (value != null && !value.isBlank()) {
                return true;
            }
        }
        return false;
    }

    private static boolean parallelConfigured() {
        String property = System.getProperty("junit.jupiter.execution.parallel.enabled");
        if (property != null) {
            return Boolean.parseBoolean(property.trim());
        }
        try (InputStream in = Thread.currentThread().getContextClassLoader().getResourceAsStream("junit-platform.properties")) {
            if (in == null) {
                return false;
            }
            Properties properties = new Properties();
            properties.load(in);
            return Boolean.parseBoolean(properties.getProperty("junit.jupiter.execution.parallel.enabled", "false").trim());
        } catch (IOException | RuntimeException error) {
            return true;
        }
    }

    private static Path scratch(String prefix) throws IOException {
        Path dir = workspace.resolve(".sieve").resolve(session.isEmpty() ? "env-run" : "run");
        Files.createDirectories(dir);
        return Files.createTempFile(dir, prefix + "-" + ProcessHandle.current().pid() + "-", ".tmp");
    }

    /**
     * Runs {@code sieve} with output going to a log: a forked test JVM's standard streams may
     * carry the build tool's own protocol.
     */
    private static boolean call(List<String> arguments) {
        try {
            List<String> command = new ArrayList<>();
            command.add(sieve);
            command.addAll(arguments);
            File log = workspace.resolve(".sieve").resolve("agent.log").toFile();
            Process process = new ProcessBuilder(command)
                    .redirectErrorStream(true)
                    .redirectOutput(ProcessBuilder.Redirect.appendTo(log))
                    .start();
            int status = process.waitFor();
            if (status != 0) {
                System.err.println("sieve: " + arguments.get(0) + " failed with status " + status + "; see " + log);
            }
            return status == 0;
        } catch (IOException | InterruptedException error) {
            System.err.println("sieve: cannot run " + sieve + ": " + error);
            return false;
        }
    }

    /** Top-level test classes that may be dropped, decided once per JVM. */
    static synchronized Set<String> skip() {
        if (skip != null) {
            return skip;
        }
        skip = Set.of();
        context();
        if (!enabled() || !mode.equals("select") || explicit() || parallelConfigured()) {
            return skip;
        }
        try {
            Path out = scratch("decide");
            List<String> arguments = new ArrayList<>(List.of("decide", "--workspace", workspace.toString(), "--jdk", jdk(), "--out", out.toString(), "--context", context(), "--session", session));
            if (base != null && !base.isEmpty()) {
                arguments.addAll(List.of("--base", base));
            }
            if (call(arguments)) {
                Set<String> names = new TreeSet<>();
                for (String line : Files.readAllLines(out, StandardCharsets.UTF_8)) {
                    if (!line.isBlank()) {
                        names.add(line.trim());
                    }
                }
                skip = names;
            }
            Files.deleteIfExists(out);
        } catch (IOException error) {
            System.err.println("sieve: selection unavailable, running every test: " + error);
        }
        return skip;
    }

    static synchronized void dropped(String name) {
        DROPPED.add(name);
    }

    static synchronized void classStarted(String name) {
        context();
        if (active != null && !active.equals(name)) {
            parallel = true;
        }
        active = name;
        Run run = RUNS.computeIfAbsent(name, key -> new Run());
        Probe.current = STARTING.isEmpty() ? run.own : STARTING.peek();
    }

    static synchronized void classFinished(String name) {
        if (name.equals(active)) {
            // A startup that never finished, such as one on an old Boot without `ready`, ends
            // with the class that started it.
            for (Bucket bucket : STARTING) {
                if (!CLAIMABLE.contains(bucket)) {
                    RUNS.get(name).contexts.add(bucket);
                }
            }
            STARTING.clear();
            active = null;
            // Work that outlives a class, such as messages its tests sent and did not await,
            // stays with it: tests are assumed not to depend on what earlier ones left behind.
            Probe.current = RUNS.get(name).own;
        }
    }

    static synchronized void failed(String name) {
        if (name == null) {
            for (Run run : RUNS.values()) {
                run.failed = true;
            }
        } else {
            RUNS.computeIfAbsent(name, key -> new Run()).failed = true;
        }
    }

    /** Startups in progress: a Spring Boot run brackets the context it creates. */
    private static final java.util.ArrayDeque<Bucket> STARTING = new java.util.ArrayDeque<>();
    /** Buckets of Boot runs, which end when Boot reports the run ready or failed. */
    private static final Set<Bucket> BOOT = Collections.newSetFromMap(new IdentityHashMap<>());
    /** Buckets that belong to a test context, which test classes claim when they use it. */
    private static final Set<Bucket> CLAIMABLE = Collections.newSetFromMap(new IdentityHashMap<>());

    private static Bucket resumed() {
        if (!STARTING.isEmpty()) {
            return STARTING.peek();
        }
        Run run = active == null ? null : RUNS.get(active);
        return run == null ? Probe.UNATTRIBUTED : run.own;
    }

    static synchronized void bootStarting() {
        Bucket bucket = new Bucket();
        BOOT.add(bucket);
        STARTING.push(bucket);
        Probe.current = bucket;
    }

    /**
     * A Boot run that no test context claimed, such as one a test starts itself, counts for the
     * running test class.
     */
    static synchronized void bootFinished() {
        Bucket bucket = STARTING.poll();
        if (bucket != null && !CLAIMABLE.contains(bucket)) {
            adopt(bucket);
        }
        Probe.current = resumed();
    }

    private static void adopt(Bucket bucket) {
        Run run = active == null ? null : RUNS.get(active);
        if (run != null) {
            run.contexts.add(bucket);
        } else {
            UNCLAIMED.add(bucket);
        }
    }

    /** Startup work outside any test class, which then counts for every test class. */
    private static final List<Bucket> UNCLAIMED = new ArrayList<>();

    /** Work done while a context starts counts for every test class that uses the context. */
    static synchronized void contextStarting(Object context) {
        Bucket bucket = STARTING.peek();
        if (bucket == null || !BOOT.contains(bucket) || CLAIMABLE.contains(bucket)) {
            bucket = new Bucket();
            STARTING.push(bucket);
        }
        CLAIMABLE.add(bucket);
        CONTEXTS.put(context, bucket);
        Probe.current = bucket;
    }

    static synchronized void contextStarted(Object context) {
        Bucket bucket = CONTEXTS.get(context);
        if (bucket != null && !BOOT.contains(bucket) && STARTING.peek() == bucket) {
            STARTING.pop();
            Probe.current = resumed();
        }
    }

    static synchronized void usesContext(String testClass, Object context) {
        Run run = RUNS.computeIfAbsent(top(testClass), key -> new Run());
        run.spring = true;
        Bucket bucket = CONTEXTS.get(context);
        if (bucket != null) {
            run.contexts.add(bucket);
        }
    }

    /** Writes the classes finished since the last flush and has {@code sieve} record them. */
    static synchronized void flush() {
        if (!enabled()) {
            return;
        }
        try {
            Path raw = scratch("raw");
            Files.writeString(raw, json(), StandardCharsets.UTF_8);
            call(List.of("record", "--workspace", workspace.toString(), "--raw", raw.toString()));
            Files.deleteIfExists(raw);
        } catch (IOException error) {
            System.err.println("sieve: cannot write test records: " + error);
        }
        FLUSHED.addAll(RUNS.keySet());
    }

    private static String json() {
        String[] names;
        synchronized (METHODS) {
            names = METHODS.toArray(String[]::new);
        }
        List<String> errors = new ArrayList<>(ERRORS);
        if (INSTRUMENTED.get() == 0 && !RUNS.isEmpty()) {
            // Tests ran, yet no project class was probed: the output directories did not match.
            errors.add("no project class was instrumented");
        }
        StringBuilder out = new StringBuilder("{\"jdk\":");
        string(out, jdk());
        out.append(",\"context\":");
        string(out, context());
        out.append(",\"session\":");
        string(out, session);
        out.append(",\"started\":").append(ProcessHandle.current().info().startInstant().map(java.time.Instant::toEpochMilli).orElse(0L));
        out.append(",\"parallel\":").append(parallel || Probe.failed || parallelConfigured());
        out.append(",\"errors\":");
        strings(out, errors);
        out.append(",\"dropped\":");
        strings(out, new ArrayList<>(DROPPED));
        out.append(",\"tests\":[");
        boolean first = true;
        for (Map.Entry<String, Run> entry : RUNS.entrySet()) {
            if (FLUSHED.contains(entry.getKey()) || entry.getKey().equals(active)) {
                continue;
            }
            Run run = entry.getValue();
            List<Bucket> buckets = new ArrayList<>(run.contexts);
            buckets.addAll(UNCLAIMED);
            buckets.add(run.own);
            buckets.add(Probe.UNATTRIBUTED);
            Set<String> methods = new TreeSet<>();
            Set<String> files = new TreeSet<>();
            for (Bucket bucket : buckets) {
                boolean[] seen = bucket.methods();
                for (int i = 0; i < seen.length && i < names.length; i++) {
                    if (seen[i]) {
                        methods.add(names[i]);
                    }
                }
                files.addAll(bucket.files());
            }
            out.append(first ? "" : ",").append("{\"name\":");
            first = false;
            string(out, entry.getKey());
            out.append(",\"passed\":").append(!run.failed);
            out.append(",\"spring\":").append(run.spring);
            out.append(",\"methods\":");
            strings(out, new ArrayList<>(methods));
            out.append(",\"files\":");
            strings(out, new ArrayList<>(files));
            out.append('}');
        }
        return out.append("]}\n").toString();
    }

    private static void strings(StringBuilder out, List<String> values) {
        out.append('[');
        for (int i = 0; i < values.size(); i++) {
            if (i > 0) {
                out.append(',');
            }
            string(out, values.get(i));
        }
        out.append(']');
    }

    private static void string(StringBuilder out, String value) {
        out.append('"');
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
                }
            }
        }
        out.append('"');
    }
}
