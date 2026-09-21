package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class ResourceIT {
@Test void resource() throws Exception {
 var props = new java.util.Properties();
 try (var stream = getClass().getResourceAsStream("/application.properties")) {
 assertNotNull(stream); props.load(stream);
 }
 assertEquals("hello", props.getProperty("greeting"));
}
}
