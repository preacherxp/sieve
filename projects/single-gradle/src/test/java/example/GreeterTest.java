package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class GreeterTest {
    @Test void greet() throws Exception {
        Greeter greeter = (Greeter) Class.forName("example.EnglishGreeter").getDeclaredConstructor().newInstance();
        assertEquals("Hello Ada", greeter.greet("Ada"));
    }
}
