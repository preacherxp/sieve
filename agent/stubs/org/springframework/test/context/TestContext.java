package org.springframework.test.context;

public interface TestContext {
    boolean hasApplicationContext();
    org.springframework.context.ApplicationContext getApplicationContext();
    Class<?> getTestClass();
}
