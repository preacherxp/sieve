package example;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import static org.junit.jupiter.api.Assertions.assertEquals;

class WelcomeIT {
    @ParameterizedTest
    @ValueSource(strings = {"Ada", "Grace"})
    void discoversRuntimeProvider(String name) {
        assertEquals("Hello " + name, Welcome.message(name));
    }
}
