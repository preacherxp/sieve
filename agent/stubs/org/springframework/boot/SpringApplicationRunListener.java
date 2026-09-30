package org.springframework.boot;

import java.time.Duration;
import org.springframework.context.ConfigurableApplicationContext;

public interface SpringApplicationRunListener {
    default void starting(ConfigurableBootstrapContext bootstrapContext) {}
    default void ready(ConfigurableApplicationContext context, Duration timeTaken) {}
    default void failed(ConfigurableApplicationContext context, Throwable exception) {}
}
