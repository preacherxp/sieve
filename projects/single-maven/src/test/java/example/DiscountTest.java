package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class DiscountTest {
    @Test void apply() { assertEquals(8, new Discount().apply(10, 2)); }
}
