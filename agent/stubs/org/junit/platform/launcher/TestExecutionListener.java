package org.junit.platform.launcher;

import org.junit.platform.engine.TestExecutionResult;

public interface TestExecutionListener {
    default void testPlanExecutionStarted(TestPlan testPlan) {}
    default void testPlanExecutionFinished(TestPlan testPlan) {}
    default void executionStarted(TestIdentifier testIdentifier) {}
    default void executionFinished(TestIdentifier testIdentifier, TestExecutionResult result) {}
}
