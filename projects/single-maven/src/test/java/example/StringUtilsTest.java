package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class StringUtilsTest {
    @Test void upper() { assertEquals("HELLO", new StringUtils().upper("hello")); }
    @Test void isEmpty() { assertTrue(new StringUtils().isEmpty("")); }
}
