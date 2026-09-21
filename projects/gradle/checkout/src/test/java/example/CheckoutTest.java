package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
public class CheckoutTest extends BaseCheckoutFixture {
 @Test void checkout() { assertEquals("card:120", new CheckoutService(new CardGateway()).checkout(amount())); }
}
