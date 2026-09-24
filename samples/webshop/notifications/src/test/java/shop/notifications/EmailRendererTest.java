package shop.notifications;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import shop.common.Money;
import shop.common.events.OrderPlaced;

class EmailRendererTest {
    @Test
    void rendersConfirmation() {
        var email = new EmailRenderer().confirmation(
            new OrderPlaced("0123456789abcdef", "BOOK-1", "Reactive Spring", 2, Money.of(7558), "ada@example.com"));
        assertThat(email.to()).isEqualTo("ada@example.com");
        assertThat(email.subject()).isEqualTo("Order 01234567 confirmed");
        assertThat(email.body()).contains("2 x Reactive Spring").contains("Total: EUR 75.58");
    }
}
