package shop.notifications;

import static org.assertj.core.api.Assertions.assertThat;

import org.junit.jupiter.api.Test;
import reactor.test.StepVerifier;
import shop.common.Money;
import shop.common.events.OrderPlaced;

class NotificationServiceTest {
    private final Outbox outbox = new Outbox();
    private final NotificationService service = new NotificationService(new EmailRenderer(), outbox);

    @Test
    void sendsOncePerOrder() {
        var event = new OrderPlaced("order-0001", "MUG-1", "Lambda mug", 1, Money.of(1500), "ada@example.com");
        StepVerifier.create(service.orderPlaced(event).then(service.orderPlaced(event))).verifyComplete();
        assertThat(outbox.sent()).hasSize(1);
    }
}
