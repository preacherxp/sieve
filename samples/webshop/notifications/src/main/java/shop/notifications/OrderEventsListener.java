package shop.notifications;

import java.time.Duration;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.boot.autoconfigure.condition.ConditionalOnProperty;
import org.springframework.boot.context.event.ApplicationReadyEvent;
import org.springframework.context.event.EventListener;
import org.springframework.core.ParameterizedTypeReference;
import org.springframework.http.MediaType;
import org.springframework.http.codec.ServerSentEvent;
import org.springframework.stereotype.Component;
import org.springframework.web.reactive.function.client.WebClient;
import reactor.core.Disposable;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;
import reactor.util.retry.Retry;
import shop.common.events.OrderPlaced;

/** Follows the orders event stream and sends one confirmation per order, reconnecting when the stream drops. */
@Component
@ConditionalOnProperty(name = "notifications.listen", havingValue = "true")
public class OrderEventsListener {
    private static final ParameterizedTypeReference<ServerSentEvent<OrderPlaced>> EVENT = new ParameterizedTypeReference<>() {
    };

    private final WebClient client;
    private final NotificationService notifications;
    private Disposable subscription;

    public OrderEventsListener(WebClient.Builder builder, @Value("${services.orders}") String ordersUrl,
                               NotificationService notifications) {
        this.client = builder.baseUrl(ordersUrl).build();
        this.notifications = notifications;
    }

    @EventListener(ApplicationReadyEvent.class)
    public void start() {
        subscription = events()
            .concatMap(notifications::orderPlaced)
            .retryWhen(Retry.backoff(Long.MAX_VALUE, Duration.ofMillis(200)).maxBackoff(Duration.ofSeconds(5)).transientErrors(true))
            .subscribe();
    }

    Flux<OrderPlaced> events() {
        return client.get().uri("/orders/events").accept(MediaType.TEXT_EVENT_STREAM)
            .retrieve()
            .bodyToFlux(EVENT)
            .mapNotNull(ServerSentEvent::data)
            .concatWith(Mono.error(new IllegalStateException("event stream closed")));
    }

    @jakarta.annotation.PreDestroy
    void stop() {
        if (subscription != null) {
            subscription.dispose();
        }
    }
}
