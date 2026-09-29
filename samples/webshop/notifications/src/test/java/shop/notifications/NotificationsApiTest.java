package shop.notifications;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.web.reactive.server.WebTestClient;

@SpringBootTest(webEnvironment = SpringBootTest.WebEnvironment.RANDOM_PORT, properties = "notifications.listen=false")
class NotificationsApiTest {
    @Autowired
    WebTestClient client;

    @Test
    void startsWithEmptyOutbox() {
        client.get().uri("/sent").exchange()
            .expectStatus().isOk()
            .expectHeader().exists("X-Correlation-Id")
            .expectBody().jsonPath("$.length()").isEqualTo(0);
    }
}
