package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class GatewayTest {
@Test void charge() { PaymentGateway gateway = new CardGateway(); assertEquals("card:100", gateway.charge(100)); }
}
