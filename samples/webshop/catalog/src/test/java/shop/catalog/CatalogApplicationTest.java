package shop.catalog;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.web.reactive.server.WebTestClient;

@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT)
class CatalogApplicationTest {
    @Autowired
    WebTestClient client;

    @Test
    void servesProductsWithCorrelationId() {
        client.get().uri("/products/KBD-1").header("X-Correlation-Id", "trace-1").exchange()
            .expectStatus().isOk()
            .expectHeader().valueEquals("X-Correlation-Id", "trace-1")
            .expectBody().jsonPath("$.category").isEqualTo("hardware");
    }

    @Test
    void listsEveryProduct() {
        client.get().uri("/products").exchange()
            .expectStatus().isOk()
            .expectBody().jsonPath("$.length()").isEqualTo(4);
    }
}
