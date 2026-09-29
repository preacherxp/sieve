package shop.orders;

import static org.mockito.ArgumentMatchers.any;
import static org.mockito.Mockito.when;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.autoconfigure.web.reactive.WebFluxTest;
import org.springframework.context.annotation.Import;
import org.springframework.http.HttpStatus;
import org.springframework.test.context.bean.override.mockito.MockitoBean;
import org.springframework.test.web.reactive.server.WebTestClient;
import reactor.core.publisher.Mono;
import shop.common.Money;
import shop.orders.clients.DownstreamException;

@WebFluxTest(OrderController.class)
@Import({OrderRepository.class, OrderEvents.class})
class OrderControllerTest {
    @Autowired
    WebTestClient client;

    @MockitoBean
    OrderService service;

    @Test
    void placesOrder() {
        when(service.place(any())).thenReturn(Mono.just(
            new Order("o-1", "MUG-1", 2, "ada@example.com", Money.of(2550), "category", OrderStatus.PLACED)));
        client.post().uri("/orders").bodyValue(new OrderRequest("MUG-1", 2, "ada@example.com")).exchange()
            .expectStatus().isCreated()
            .expectBody().jsonPath("$.total.cents").isEqualTo(2550);
    }

    @Test
    void rejectsInvalidOrder() {
        client.post().uri("/orders").bodyValue(new OrderRequest("MUG-1", 0, "nobody")).exchange()
            .expectStatus().isBadRequest()
            .expectBody().jsonPath("$.code").isEqualTo("INVALID_ORDER");
    }

    @Test
    void mapsDownstreamErrors() {
        when(service.place(any()))
            .thenReturn(Mono.error(new DownstreamException("catalog", HttpStatus.NOT_FOUND)))
            .thenReturn(Mono.error(new DownstreamException("inventory", HttpStatus.CONFLICT)))
            .thenReturn(Mono.error(new DownstreamException("pricing", HttpStatus.SERVICE_UNAVAILABLE)));
        var request = new OrderRequest("MUG-1", 2, "ada@example.com");
        client.post().uri("/orders").bodyValue(request).exchange()
            .expectStatus().isNotFound().expectBody().jsonPath("$.code").isEqualTo("PRODUCT_NOT_FOUND");
        client.post().uri("/orders").bodyValue(request).exchange()
            .expectStatus().isEqualTo(409).expectBody().jsonPath("$.code").isEqualTo("OUT_OF_STOCK");
        client.post().uri("/orders").bodyValue(request).exchange()
            .expectStatus().isEqualTo(502);
    }

    @Test
    void unknownOrderIsNotFound() {
        client.get().uri("/orders/missing").exchange().expectStatus().isNotFound();
    }
}
