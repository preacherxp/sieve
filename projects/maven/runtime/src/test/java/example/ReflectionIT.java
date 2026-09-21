package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class ReflectionIT {
@Test void reflection() throws Exception {
 var type = Class.forName("example.ReflectivePlugin");
 assertEquals("reflective", type.getMethod("value").invoke(type.getConstructor().newInstance()));
}
}
