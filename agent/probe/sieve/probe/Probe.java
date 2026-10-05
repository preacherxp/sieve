package sieve.probe;

import java.io.File;
import java.nio.file.Path;

/**
 * Called by instrumented code. Loaded by the bootstrap class loader, so that JDK file APIs and
 * project classes in any class loader reach the same state. It never touches files itself.
 */
public final class Probe {
    /** Everything not attributed to a test class or a context startup. */
    public static final Bucket UNATTRIBUTED = new Bucket();

    public static volatile Bucket current = UNATTRIBUTED;

    /** Absolute workspace path ending in a separator; file reads elsewhere are ignored. */
    public static volatile String root;

    /** The workspace's real path, when a symlink or another spelling leads to it. */
    public static volatile String realRoot;

    /** Set when a read may have gone unrecorded; the run then keeps no records. */
    public static volatile boolean failed;

    private Probe() {}

    public static void roots(Path workspace) {
        String absolute = workspace.toAbsolutePath().toString();
        String real = absolute;
        try {
            real = workspace.toRealPath().toString();
        } catch (java.io.IOException ignored) {
            // The absolute path is all there is.
        }
        realRoot = real.endsWith(File.separator) ? real : real + File.separator;
        root = absolute.endsWith(File.separator) ? absolute : absolute + File.separator;
    }

    public static void hit(int id) {
        current.hit(id);
    }

    public static void file(Object target) {
        record(target, "");
    }

    /** A directory listing, whose entries count rather than the directory's existence. */
    public static void list(Object target) {
        record(target, LISTED);
    }

    /** Marks listed directories in the recorded paths. */
    public static final String LISTED = "ls:";

    /** Marks a library jar, in method names and recorded paths. */
    public static final String JAR = "jar:";

    /** Counts for every test class: a jar whose use cannot be attributed. */
    public static void always(int id) {
        UNATTRIBUTED.hit(id);
    }

    /**
     * Entries of a jar outside the workspace were read, as class bytes or resources. Jars the
     * build packaged inside the workspace are project code.
     */
    public static void jar(Object target) {
        String root = Probe.root;
        if (root == null || !(target instanceof java.util.zip.ZipFile zip)) {
            return;
        }
        try {
            String path = zip.getName();
            String real = realRoot;
            boolean inside = path.startsWith(root) || real != null && path.startsWith(real);
            if (path.endsWith(".jar") && !inside) {
                current.file(JAR + path);
            }
        } catch (Throwable error) {
            failed = true;
        }
    }

    private static void record(Object target, String kind) {
        String root = Probe.root;
        if (root == null || target == null) {
            return;
        }
        try {
            String path;
            if (target instanceof File file) {
                path = file.getAbsolutePath();
            } else if (target instanceof Path p) {
                if (!"file".equals(p.getFileSystem().provider().getScheme())) {
                    return;
                }
                path = p.toAbsolutePath().toString();
            } else if (target instanceof String name) {
                path = new File(name).getAbsolutePath();
            } else {
                return;
            }
            String real = realRoot;
            boolean inside = path.startsWith(root) || real != null && path.startsWith(real);
            if (inside && !path.endsWith(".class")) {
                current.file(kind + path);
            }
        } catch (Throwable error) {
            // A probe must never change the behavior of the code it observes.
            failed = true;
        }
    }
}
