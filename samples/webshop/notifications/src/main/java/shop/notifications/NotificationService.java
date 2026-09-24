package shop.notifications;

import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import org.springframework.stereotype.Service;
import reactor.core.publisher.Mono;
import shop.common.events.OrderPlaced;

/** Sends at most one confirmation per order, since a reconnecting stream replays recent events. */
@Service
public class NotificationService {
    private final EmailRenderer renderer;
    private final Outbox outbox;
    private final Set<String> notified = ConcurrentHashMap.newKeySet();

    public NotificationService(EmailRenderer renderer, Outbox outbox) {
        this.renderer = renderer;
        this.outbox = outbox;
    }

    public Mono<Email> orderPlaced(OrderPlaced event) {
        if (!notified.add(event.orderId())) {
            return Mono.empty();
        }
        return outbox.send(renderer.confirmation(event));
    }
}
