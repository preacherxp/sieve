package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class PriceCalculatorTest {
@Test void total() { assertEquals(120, new PriceCalculator().total(100)); }
}
