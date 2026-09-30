package sieve.agent.spring;

import java.time.Duration;
import org.springframework.boot.ConfigurableBootstrapContext;
import org.springframework.boot.SpringApplicationRunListener;
import org.springframework.context.ConfigurableApplicationContext;
import sieve.agent.Contexts;

/**
 * Brackets a Spring Boot startup from its first step: Boot reads its configuration files
 * before any context customizer runs, and calls runners after the context has refreshed.
 */
public final class BootRun implements SpringApplicationRunListener {
    @Override
    public void starting(ConfigurableBootstrapContext bootstrapContext) {
        Contexts.bootStarting();
    }

    @Override
    public void ready(ConfigurableApplicationContext context, Duration timeTaken) {
        Contexts.bootFinished();
    }

    @Override
    public void failed(ConfigurableApplicationContext context, Throwable exception) {
        Contexts.bootFinished();
    }
}
