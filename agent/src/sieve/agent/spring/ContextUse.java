package sieve.agent.spring;

import org.springframework.context.ApplicationContext;
import org.springframework.test.context.TestContext;
import org.springframework.test.context.TestExecutionListener;
import sieve.agent.Contexts;

/** Links each test class to the Spring context it ran against, cached or not. */
public final class ContextUse implements TestExecutionListener {
    @Override
    public void prepareTestInstance(TestContext testContext) {
        see(testContext);
    }

    @Override
    public void afterTestClass(TestContext testContext) {
        see(testContext);
    }

    private static void see(TestContext testContext) {
        if (testContext.hasApplicationContext()) {
            // A context hierarchy's parents started for this test class too.
            for (ApplicationContext context = testContext.getApplicationContext(); context != null; context = context.getParent()) {
                Contexts.used(testContext.getTestClass().getName(), context);
            }
        }
    }
}
