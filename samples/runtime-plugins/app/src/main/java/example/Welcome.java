package example;

import java.util.ServiceLoader;

public final class Welcome {
    public static String message(String name) {
        return ServiceLoader.load(Greeting.class).findFirst().orElseThrow().greet(name);
    }
}
