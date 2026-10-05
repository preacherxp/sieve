package example;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.Properties;

public final class EnglishGreeting implements Greeting {
    public String greet(String name) {
        var properties = new Properties();
        try (var input = getClass().getResourceAsStream("/greeting.properties")) {
            properties.load(input);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return properties.getProperty("prefix") + " " + name;
    }
}
