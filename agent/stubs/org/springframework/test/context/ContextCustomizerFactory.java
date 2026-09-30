package org.springframework.test.context;

public interface ContextCustomizerFactory {
    ContextCustomizer createContextCustomizer(Class<?> testClass, java.util.List<ContextConfigurationAttributes> configAttributes);
}
