package shop.orders;

import org.springframework.stereotype.Component;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Sinks;
import shop.common.events.OrderPlaced;

/** Hot stream of placed orders; late subscribers get the most recent events too. */
@Component
public class OrderEvents {
    private final Sinks.Many<OrderPlaced> sink = Sinks.many().replay().limit(100);

    public void publish(OrderPlaced event) {
        sink.emitNext(event, Sinks.EmitFailureHandler.busyLooping(java.time.Duration.ofSeconds(1)));
    }

    public Flux<OrderPlaced> stream() {
        return sink.asFlux();
    }
}
