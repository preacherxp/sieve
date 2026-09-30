package org.junit.platform.engine;

public class TestExecutionResult {
    public enum Status { SUCCESSFUL, ABORTED, FAILED }
    public Status getStatus() { throw new UnsupportedOperationException(); }
}
