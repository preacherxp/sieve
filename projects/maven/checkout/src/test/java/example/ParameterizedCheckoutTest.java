package example;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;
import static org.junit.jupiter.api.Assertions.*;
public class ParameterizedCheckoutTest extends BaseCheckoutFixture {
 @ParameterizedTest @ValueSource(ints={1,2}) void checkout(int count) {
 assertEquals("card:" + (120 * count), new CheckoutService(new CardGateway()).checkout(amount() * count));
 }
}
