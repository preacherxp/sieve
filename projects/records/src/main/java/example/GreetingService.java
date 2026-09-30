package example;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Service;

@Service
public class GreetingService {
    private final String prefix;

    public GreetingService(@Value("${app.greeting.prefix}") String prefix) {
        this.prefix = prefix;
    }

    public String greeting(String name) {
        return prefix + ", " + name;
    }
}
