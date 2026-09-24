package shop.orders;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import org.springframework.stereotype.Repository;
import reactor.core.publisher.Mono;

@Repository
public class OrderRepository {
    private final Map<String, Order> orders = new ConcurrentHashMap<>();

    public Mono<Order> save(Order order) {
        return Mono.fromSupplier(() -> {
            orders.put(order.id(), order);
            return order;
        });
    }

    public Mono<Order> findById(String id) {
        return Mono.justOrEmpty(orders.get(id));
    }
}
