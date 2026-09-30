package org.springframework.test.context;

public interface ContextCustomizer {
    void customizeContext(org.springframework.context.ConfigurableApplicationContext context, MergedContextConfiguration mergedConfig);
}
