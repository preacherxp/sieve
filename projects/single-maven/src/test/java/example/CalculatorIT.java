package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class CalculatorIT {
    @Test void chain() { assertEquals(4, new Calculator().subtract(new Calculator().add(3, 3), 2)); }
}
