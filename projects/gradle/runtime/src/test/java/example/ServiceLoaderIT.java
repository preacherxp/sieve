package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class ServiceLoaderIT {
@Test void service() {
 var greeting = java.util.ServiceLoader.load(Greeting.class).findFirst().orElseThrow();
 assertEquals("hello", greeting.message());
}
}
