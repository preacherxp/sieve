package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class CurrencyLabelTest {
@Test void label() { assertEquals("EUR", new CurrencyLabel().label()); }
}
