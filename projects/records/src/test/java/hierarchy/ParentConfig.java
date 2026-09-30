package hierarchy;

import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;

/** The parent of a context hierarchy, outside the application's scanned package. */
@Configuration
public class ParentConfig {
    @Bean
    public String word() {
        return new StringBuilder("parent").toString();
    }
}
