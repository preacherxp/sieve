package shop.orders;

import java.util.UUID;
import org.springframework.stereotype.Service;
import reactor.core.publisher.Mono;
import shop.common.events.OrderPlaced;
import shop.orders.clients.CatalogClient;
import shop.orders.clients.InventoryClient;
import shop.orders.clients.PricingClient;

/** Looks up and prices the product, reserves stock, then records and announces the order. */
@Service
public class OrderService {
    private final CatalogClient catalog;
    private final PricingClient pricing;
    private final InventoryClient inventory;
    private final OrderRepository repository;
    private final OrderEvents events;

    public OrderService(CatalogClient catalog, PricingClient pricing, InventoryClient inventory,
                        OrderRepository repository, OrderEvents events) {
        this.catalog = catalog;
        this.pricing = pricing;
        this.inventory = inventory;
        this.repository = repository;
        this.events = events;
    }

    public Mono<Order> place(OrderRequest request) {
        return catalog.product(request.sku())
            .flatMap(product -> pricing.quote(product, request.quantity())
                .zipWith(inventory.reserve(product.sku(), request.quantity()))
                .flatMap(priced -> {
                    var quote = priced.getT1();
                    var reservation = priced.getT2();
                    var order = new Order(UUID.randomUUID().toString(), product.sku(), request.quantity(),
                        request.email(), quote.total(), quote.rule(), OrderStatus.PLACED);
                    return inventory.confirm(reservation.id())
                        .then(repository.save(order))
                        .doOnNext(saved -> events.publish(new OrderPlaced(saved.id(), saved.sku(), product.name(),
                            saved.quantity(), saved.total(), saved.email())));
                }));
    }
}
