package org.springframework.context;

public interface ConfigurableApplicationContext extends ApplicationContext {
    void addApplicationListener(ApplicationListener<?> listener);
}
