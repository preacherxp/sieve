package org.springframework.test.context;

public interface TestExecutionListener {
    default void prepareTestInstance(TestContext testContext) throws Exception {}
    default void afterTestClass(TestContext testContext) throws Exception {}
}
