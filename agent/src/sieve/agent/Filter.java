package sieve.agent;

import java.util.Set;
import org.junit.platform.engine.FilterResult;
import org.junit.platform.engine.TestDescriptor;
import org.junit.platform.engine.TestSource;
import org.junit.platform.engine.support.descriptor.ClassSource;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.PostDiscoveryFilter;

/**
 * Drops the test classes whose records show that no change can affect them. JUnit removes only
 * excluded leaves, so tests are matched by the class that declares them, and emptied classes
 * are pruned.
 */
public final class Filter implements PostDiscoveryFilter {
    @Override
    public FilterResult apply(TestDescriptor descriptor) {
        try {
            TestSource source = descriptor.getSource().orElse(null);
            String name = source instanceof ClassSource c ? c.getClassName()
                    : source instanceof MethodSource m ? m.getClassName() : null;
            if (name != null && State.enabled()) {
                String top = State.top(name);
                Set<String> skip = State.skip();
                if (skip.contains(top)) {
                    State.dropped(top);
                    return FilterResult.excluded("sieve: unchanged test record");
                }
            }
        } catch (RuntimeException error) {
            State.error("filter: " + error);
        }
        return FilterResult.included(null);
    }
}
