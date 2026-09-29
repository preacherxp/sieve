package shop.catalog;

import org.junit.jupiter.api.Test;
import org.springframework.test.web.reactive.server.WebTestClient;

/** Exercises the functional routes without starting Spring. */
class CatalogRoutesTest {
    private final WebTestClient client = WebTestClient
        .bindToRouterFunction(new CatalogRouter().catalogRoutes(new CatalogHandler(new ProductRepository())))
        .build();

    @Test
    void getsProduct() {
        client.get().uri("/products/BOOK-1").exchange()
            .expectStatus().isOk()
            .expectBody()
            .jsonPath("$.name").isEqualTo("Reactive Spring")
            .jsonPath("$.price.cents").isEqualTo(3999);
    }

    @Test
    void unknownProductIsNotFound() {
        client.get().uri("/products/NOPE").exchange()
            .expectStatus().isNotFound()
            .expectBody().jsonPath("$.code").isEqualTo("PRODUCT_NOT_FOUND");
    }

    @Test
    void filtersByCategory() {
        client.get().uri("/products?category=merch").exchange()
            .expectStatus().isOk()
            .expectBody().jsonPath("$.length()").isEqualTo(1);
    }
}
