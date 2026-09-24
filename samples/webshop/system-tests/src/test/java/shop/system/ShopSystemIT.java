package shop.system;

import static org.assertj.core.api.Assertions.assertThat;

import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.stream.Stream;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.springframework.boot.WebApplicationType;
import org.springframework.boot.builder.SpringApplicationBuilder;
import org.springframework.boot.web.context.WebServerApplicationContext;
import org.springframework.context.ConfigurableApplicationContext;
import org.springframework.core.ParameterizedTypeReference;
import org.springframework.test.web.reactive.server.WebTestClient;
import shop.catalog.CatalogApplication;
import shop.inventory.InventoryApplication;
import shop.notifications.NotificationsApplication;
import shop.orders.OrdersApplication;
import shop.pricing.PricingApplication;

/** Boots every service on a random port, wired to each other over HTTP, and drives a purchase end to end. */
class ShopSystemIT {
    static ConfigurableApplicationContext catalog;
    static ConfigurableApplicationContext pricing;
    static ConfigurableApplicationContext inventory;
    static ConfigurableApplicationContext orders;
    static ConfigurableApplicationContext notifications;
    static WebTestClient ordersClient;
    static WebTestClient notificationsClient;

    /** Arguments, unlike builder properties, override each service's application.properties. */
    static ConfigurableApplicationContext start(Class<?> application, String name, String... properties) {
        var args = Stream.concat(Stream.of("server.port=0", "spring.application.name=" + name), Stream.of(properties))
            .map(property -> "--" + property)
            .toArray(String[]::new);
        return new SpringApplicationBuilder(application).web(WebApplicationType.REACTIVE).run(args);
    }

    static String url(ConfigurableApplicationContext context) {
        return "http://localhost:" + ((WebServerApplicationContext) context).getWebServer().getPort();
    }

    static WebTestClient client(ConfigurableApplicationContext context) {
        return WebTestClient.bindToServer().baseUrl(url(context)).responseTimeout(Duration.ofSeconds(10)).build();
    }

    @BeforeAll
    static void startShop() {
        catalog = start(CatalogApplication.class, "catalog");
        pricing = start(PricingApplication.class, "pricing");
        inventory = start(InventoryApplication.class, "inventory");
        orders = start(OrdersApplication.class, "orders",
            "services.catalog=" + url(catalog), "services.pricing=" + url(pricing), "services.inventory=" + url(inventory));
        notifications = start(NotificationsApplication.class, "notifications",
            "services.orders=" + url(orders), "notifications.listen=true");
        ordersClient = client(orders);
        notificationsClient = client(notifications);
    }

    @AfterAll
    static void stopShop() {
        Stream.of(notifications, orders, inventory, pricing, catalog).filter(Objects::nonNull).forEach(ConfigurableApplicationContext::close);
    }

    @Test
    void purchaseIsPricedReservedAndConfirmedByEmail() throws InterruptedException {
        ordersClient.post().uri("/orders")
            .bodyValue(Map.of("sku", "MUG-1", "quantity", 12, "email", "ada@example.com"))
            .exchange()
            .expectStatus().isCreated()
            .expectBody()
            .jsonPath("$.pricingRule").isEqualTo("category")
            .jsonPath("$.total.cents").isEqualTo(15300);
        inventoryHas("MUG-1", 88);
        var emails = awaitEmails("ada@example.com");
        assertThat(emails).singleElement()
            .satisfies(email -> assertThat(email.get("body").toString()).contains("12 x Lambda mug", "Total: EUR 153.00"));
    }

    @Test
    void booksGetReducedTax() {
        ordersClient.post().uri("/orders")
            .bodyValue(Map.of("sku", "BOOK-2", "quantity", 1, "email", "grace@example.com"))
            .exchange()
            .expectStatus().isCreated()
            .expectBody().jsonPath("$.total.cents").isEqualTo(4300);
    }

    @Test
    void oversellIsRejectedWithoutTakingStock() {
        ordersClient.post().uri("/orders")
            .bodyValue(Map.of("sku", "KBD-1", "quantity", 4, "email", "linus@example.com"))
            .exchange()
            .expectStatus().isEqualTo(409)
            .expectBody().jsonPath("$.code").isEqualTo("OUT_OF_STOCK");
        inventoryHas("KBD-1", 3);
    }

    @Test
    void unknownProductIsNotFound() {
        ordersClient.post().uri("/orders")
            .bodyValue(Map.of("sku", "NOPE", "quantity", 1, "email", "ada@example.com"))
            .exchange()
            .expectStatus().isNotFound();
    }

    @Test
    void correlationIdCrossesServices() {
        ordersClient.get().uri("/orders/missing").header("X-Correlation-Id", "system-1").exchange()
            .expectStatus().isNotFound()
            .expectHeader().valueEquals("X-Correlation-Id", "system-1");
    }

    private void inventoryHas(String sku, int units) {
        client(inventory).get().uri("/stock/{sku}", sku).exchange()
            .expectBody().jsonPath("$.available").isEqualTo(units);
    }

    private List<Map<String, Object>> awaitEmails(String to) throws InterruptedException {
        var type = new ParameterizedTypeReference<List<Map<String, Object>>>() {
        };
        long deadline = System.nanoTime() + Duration.ofSeconds(10).toNanos();
        while (true) {
            var sent = notificationsClient.get().uri("/sent").exchange().expectBody(type).returnResult().getResponseBody();
            var matching = sent.stream().filter(email -> to.equals(email.get("to"))).toList();
            if (!matching.isEmpty() || System.nanoTime() > deadline) {
                return matching;
            }
            Thread.sleep(100);
        }
    }
}
