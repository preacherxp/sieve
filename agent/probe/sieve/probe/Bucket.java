package sieve.probe;

import java.util.Arrays;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/** The methods and files one test class, one context startup, or the unattributed rest used. */
public final class Bucket {
    private volatile boolean[] methods = new boolean[4096];
    private final Set<String> files = ConcurrentHashMap.newKeySet();
    private final Set<String> invocations = ConcurrentHashMap.newKeySet();

    void hit(int id) {
        boolean[] seen = methods;
        if (id < seen.length && seen[id]) {
            return;
        }
        mark(id);
    }

    private synchronized void mark(int id) {
        boolean[] seen = methods;
        if (id >= seen.length) {
            seen = Arrays.copyOf(seen, Math.max(id + 1, seen.length * 2));
        }
        seen[id] = true;
        methods = seen;
    }

    void file(String path) {
        files.add(path);
    }

    void invocation(String method) {
        invocations.add(method);
    }

    public Set<String> invocations() {
        return invocations;
    }

    public synchronized boolean[] methods() {
        return methods.clone();
    }

    public Set<String> files() {
        return files;
    }
}
