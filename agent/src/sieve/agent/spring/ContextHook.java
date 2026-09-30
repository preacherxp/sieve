package sieve.agent.spring;

import java.util.List;
import org.springframework.context.ApplicationListener;
import org.springframework.context.ConfigurableApplicationContext;
import org.springframework.context.event.ContextRefreshedEvent;
import org.springframework.test.context.ContextConfigurationAttributes;
import org.springframework.test.context.ContextCustomizer;
import org.springframework.test.context.ContextCustomizerFactory;
import org.springframework.test.context.MergedContextConfiguration;
import sieve.agent.Contexts;

/**
 * Marks the startup of each Spring test context. Every customizer is equal, so the context
 * cache keys stay as they would be without the agent.
 */
public final class ContextHook implements ContextCustomizerFactory {
    @Override
    public ContextCustomizer createContextCustomizer(Class<?> testClass, List<ContextConfigurationAttributes> attributes) {
        return Customizer.INSTANCE;
    }

    static final class Customizer implements ContextCustomizer {
        static final Customizer INSTANCE = new Customizer();

        @Override
        public void customizeContext(ConfigurableApplicationContext context, MergedContextConfiguration config) {
            Contexts.starting(context);
            Contexts.propertySources(config);
            context.addApplicationListener(new Refreshed(context));
        }

        @Override
        public boolean equals(Object other) {
            return other instanceof Customizer;
        }

        @Override
        public int hashCode() {
            return Customizer.class.getName().hashCode();
        }
    }

    static final class Refreshed implements ApplicationListener<ContextRefreshedEvent> {
        private final Object context;

        Refreshed(Object context) {
            this.context = context;
        }

        @Override
        public void onApplicationEvent(ContextRefreshedEvent event) {
            if (event.getSource() == context) {
                Contexts.started(context);
            }
        }
    }
}
