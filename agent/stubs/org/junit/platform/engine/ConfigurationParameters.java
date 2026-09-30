package org.junit.platform.engine;

public interface ConfigurationParameters {
    java.util.Optional<Boolean> getBoolean(String key);
}
