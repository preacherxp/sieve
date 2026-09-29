package shop.orders;

import java.time.Duration;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.http.MediaType;
import org.springframework.test.context.DynamicPropertyRegistry;
import org.springframework.test.context.DynamicPropertySource;
import org.springframework.test.web.reactive.server.WebTestClient;
import reactor.core.publisher.Mono;
import reactor.netty.DisposableServer;
import reactor.netty.http.server.HttpServer;
import reactor.test.StepVerifier;
import shop.common.events.OrderPlaced;

/** Runs the whole orders service against stub HTTP servers standing in for its dependencies. */
@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT)
class OrderFlowIT {
    static final DisposableServer DOWNSTREAM = HttpServer.create().port(0).route(routes -> routes
        .get("/products/KBD-1", (request, response) -> json(response, """
            {"sku":"KBD-1","name":"Mechanical keyboard","category":"hardware","price":{"cents":12900,"currency":"EUR"}}"""))
        .get("/products/{sku}", (request, response) -> response.status(404).send())
        .post("/quotes", (request, response) -> json(response, """
            {"subtotal":{"cents":12900,"currency":"EUR"},"discount":{"cents":0,"currency":"EUR"},"rule":"none",
             "tax":{"cents":2580,"currency":"EUR"},"total":{"cents":15480,"currency":"EUR"}}"""))
        .post("/reservations", (request, response) -> json(response.status(201), """
            {"id":"r-1","sku":"KBD-1","quantity":1,"confirmed":false}"""))
        .post("/reservations/r-1/confirm", (request, response) -> json(response, """
            {"id":"r-1","sku":"KBD-1","quantity":1,"confirmed":true}""")))
        .bindNow();

    static reactor.netty.NettyOutbound json(reactor.netty.http.server.HttpServerResponse response, String body) {
        return response.header("Content-Type", "application/json").sendString(Mono.just(body));
    }

    @DynamicPropertySource
    static void downstream(DynamicPropertyRegistry registry) {
        String url = "http://localhost:" + DOWNSTREAM.port();
        registry.add("services.catalog", () -> url);
        registry.add("services.pricing", () -> url);
        registry.add("services.inventory", () -> url);
    }

    @AfterAll
    static void stop() {
        DOWNSTREAM.disposeNow();
    }

    @Autowired
    WebTestClient client;

    @Test
    void placesOrderAndStreamsEvent() {
        var order = client.post().uri("/orders").bodyValue(new OrderRequest("KBD-1", 1, "ada@example.com")).exchange()
            .expectStatus().isCreated()
            .expectBody(Order.class).returnResult().getResponseBody();
        client.get().uri("/orders/{id}", order.id()).exchange().expectStatus().isOk();
        var events = client.get().uri("/orders/events").accept(MediaType.TEXT_EVENT_STREAM).exchange()
            .expectStatus().isOk()
            .returnResult(OrderPlaced.class).getResponseBody();
        StepVerifier.create(events.take(1))
            .expectNextMatches(event -> event.orderId().equals(order.id()) && event.total().cents() == 15480)
            .expectComplete()
            .verify(Duration.ofSeconds(5));
    }

    @Test
    void unknownProductIsNotFound() {
        client.post().uri("/orders").bodyValue(new OrderRequest("NOPE", 1, "ada@example.com")).exchange()
            .expectStatus().isNotFound()
            .expectBody().jsonPath("$.code").isEqualTo("PRODUCT_NOT_FOUND");
    }
}
