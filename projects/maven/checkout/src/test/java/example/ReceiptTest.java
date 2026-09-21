package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class ReceiptTest {
@Test void receipt() { assertEquals("receipt", new LegacyReceipt().text()); }
}
