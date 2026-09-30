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
                current.file(path);
            }
        } catch (Throwable error) {
            // A probe must never change the behavior of the code it observes.
            failed = true;
        }
    }
}
