package shop.inventory;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicBoolean;
import org.springframework.stereotype.Component;
import reactor.core.publisher.Mono;

/** Units on hand per SKU. Reservations take units out until they are released. */
@Component
public class StockLedger {
    private final Map<String, Integer> onHand = new ConcurrentHashMap<>(Map.of(
        "BOOK-1", 25, "BOOK-2", 5, "MUG-1", 100, "KBD-1", 3));

    public Mono<Integer> available(String sku) {
        return Mono.fromSupplier(() -> onHand.getOrDefault(sku, 0));
    }

    public Mono<Boolean> take(String sku, int quantity) {
        return Mono.fromSupplier(() -> {
            var taken = new AtomicBoolean();
            onHand.computeIfPresent(sku, (key, units) -> {
                if (units < quantity) {
                    return units;
                }
                taken.set(true);
                return units - quantity;
            });
            return taken.get();
        });
    }

    public Mono<Void> put(String sku, int quantity) {
        return Mono.fromRunnable(() -> onHand.merge(sku, quantity, Integer::sum));
    }
}
