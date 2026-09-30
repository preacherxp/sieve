package sieve.agent;

import java.util.Optional;
import org.junit.platform.engine.TestExecutionResult;
import org.junit.platform.engine.TestSource;
import org.junit.platform.engine.support.descriptor.ClassSource;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.TestExecutionListener;
import org.junit.platform.launcher.TestIdentifier;
import org.junit.platform.launcher.TestPlan;

/** Attributes probe hits to the running top-level test class and records its outcome. */
public final class Listener implements TestExecutionListener {
    private volatile TestPlan plan;

    @Override
    public void testPlanExecutionStarted(TestPlan testPlan) {
        plan = testPlan;
    }

    @Override
    public void executionStarted(TestIdentifier id) {
        if (!State.enabled()) {
            return;
        }
        String name = className(id.getSource());
        if (name != null && id.isContainer()) {
            State.classStarted(State.top(name));
        }
    }

    @Override
    public void executionFinished(TestIdentifier id, TestExecutionResult result) {
        if (!State.enabled()) {
            return;
        }
        String name = owner(id);
        if (result.getStatus() != TestExecutionResult.Status.SUCCESSFUL) {
            State.failed(name == null ? null : State.top(name));
        }
        String own = className(id.getSource());
        if (own != null && id.isContainer() && own.indexOf('$') < 0) {
            State.classFinished(own);
        }
    }

    @Override
    public void testPlanExecutionFinished(TestPlan testPlan) {
        State.flush();
    }

    private String owner(TestIdentifier id) {
        TestPlan plan = this.plan;
        for (Optional<TestIdentifier> at = Optional.of(id); at.isPresent(); at = plan == null ? Optional.empty() : plan.getParent(at.get())) {
            String name = className(at.get().getSource());
            if (name != null) {
                return name;
            }
        }
        return null;
    }

    private static String className(Optional<TestSource> source) {
        if (source.isEmpty()) {
            return null;
        }
        if (source.get() instanceof ClassSource c) {
            return c.getClassName();
        }
        if (source.get() instanceof MethodSource m) {
            return m.getClassName();
        }
        return null;
    }
}
