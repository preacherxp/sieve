package sieve.agent.spring;

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
            Contexts.used(testContext.getTestClass().getName(), testContext.getApplicationContext());
        }
    }
}
