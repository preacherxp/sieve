package example;

import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.context.ApplicationEventPublisher;

@SpringBootTest
class OrderFlowTest {
    @Autowired
    ApplicationEventPublisher events;

    @Autowired
    AuditLog log;

    @Test
    void auditsPlacedOrders() throws InterruptedException {
        events.publishEvent(new OrderPlaced("42"));
        for (int i = 0; i < 100 && !log.entries().contains("placed 42"); i++) {
            Thread.sleep(20);
        }
        assertTrue(log.entries().contains("placed 42"), log.entries().toString());
    }
}
