package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class CalculatorTest {
    @Test void add() { assertEquals(5, new Calculator().add(2, 3)); }
    @Test void subtract() { assertEquals(1, new Calculator().subtract(3, 2)); }
}
