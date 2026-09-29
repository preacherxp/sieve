package shop.orders;

import static org.assertj.core.api.Assertions.assertThat;

import java.time.Duration;
import org.junit.jupiter.api.Test;
import org.springframework.http.HttpStatus;
import reactor.test.StepVerifier;
import shop.common.Money;
import shop.common.events.OrderPlaced;
import shop.orders.clients.CatalogClient;
import shop.orders.clients.DownstreamException;
import shop.orders.clients.InventoryClient;
import shop.orders.clients.PricingClient;
import shop.orders.clients.StubExchange;

class OrderServiceTest {
    private final StubExchange stub = new StubExchange()
        .on("GET /products/BOOK-1", HttpStatus.OK, """
            {"sku":"BOOK-1","name":"Reactive Spring","category":"books","price":{"cents":3999,"currency":"EUR"}}""")
        .on("POST /quotes", HttpStatus.OK, """
            {"subtotal":{"cents":7998,"currency":"EUR"},"discount":{"cents":800,"currency":"EUR"},"rule":"category",
             "tax":{"cents":360,"currency":"EUR"},"total":{"cents":7558,"currency":"EUR"}}""")
        .on("POST /reservations", HttpStatus.CREATED, """
            {"id":"r-7","sku":"BOOK-1","quantity":2,"confirmed":false}""")
        .on("POST /reservations/r-7/confirm", HttpStatus.OK, """
            {"id":"r-7","sku":"BOOK-1","quantity":2,"confirmed":true}""");
    private final OrderRepository repository = new OrderRepository();
    private final OrderEvents events = new OrderEvents();
    private final OrderService service = new OrderService(
        new CatalogClient(stub.builder(), "http://catalog"),
        new PricingClient(stub.builder(), "http://pricing"),
        new InventoryClient(stub.builder(), "http://inventory"),
        repository, events);

    @Test
    void placesConfirmsStoresAndAnnounces() {
        var order = service.place(new OrderRequest("BOOK-1", 2, "ada@example.com")).block();
        assertThat(order.total()).isEqualTo(Money.of(7558));
        assertThat(order.status()).isEqualTo(OrderStatus.PLACED);
        assertThat(stub.calls("POST /reservations/r-7/confirm")).isEqualTo(1);
        StepVerifier.create(repository.findById(order.id())).expectNext(order).verifyComplete();
        StepVerifier.create(events.stream().take(1))
            .expectNext(new OrderPlaced(order.id(), "BOOK-1", "Reactive Spring", 2, Money.of(7558), "ada@example.com"))
            .expectComplete()
            .verify(Duration.ofSeconds(5));
    }

    @Test
    void outOfStockStoresNothing() {
        var failing = new OrderService(new CatalogClient(new StubExchange()
            .on("GET /products/BOOK-1", HttpStatus.OK, """
                {"sku":"BOOK-1","name":"Reactive Spring","category":"books","price":{"cents":3999,"currency":"EUR"}}""")
            .builder(), "http://catalog"),
            new PricingClient(stub.builder(), "http://pricing"),
            new InventoryClient(new StubExchange().on("POST /reservations", HttpStatus.CONFLICT, "{}").builder(), "http://inventory"),
            repository, events);
        StepVerifier.create(failing.place(new OrderRequest("BOOK-1", 99, "ada@example.com")))
            .expectErrorMatches(e -> e instanceof DownstreamException d && d.status().value() == 409)
            .verify(Duration.ofSeconds(5));
        StepVerifier.create(events.stream().take(Duration.ofMillis(100))).verifyComplete();
    }
}
