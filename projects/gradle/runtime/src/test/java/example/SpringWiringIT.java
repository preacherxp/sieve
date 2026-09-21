package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import org.springframework.context.annotation.AnnotationConfigApplicationContext;
public class SpringWiringIT {
@Test void greeting() { try (var context = new AnnotationConfigApplicationContext(GreetingConfig.class)) {
 assertEquals("hello", context.getBean(Greeting.class).message());
} }
}
