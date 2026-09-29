package shop.orders.clients;

import java.util.Map;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.HttpStatusCode;
import org.springframework.stereotype.Component;
import org.springframework.web.reactive.function.client.WebClient;
import reactor.core.publisher.Mono;

@Component
public class InventoryClient {
    private final WebClient client;

    public InventoryClient(WebClient.Builder builder, @Value("${services.inventory}") String baseUrl) {
        this.client = builder.baseUrl(baseUrl).build();
    }

    /** Not retried: a repeated reservation could hold stock twice. */
    public Mono<ReservationView> reserve(String sku, int quantity) {
        return client.post().uri("/reservations").bodyValue(Map.of("sku", sku, "quantity", quantity))
            .retrieve()
            .onStatus(HttpStatusCode::isError, response -> Mono.error(new DownstreamException("inventory", response.statusCode())))
            .bodyToMono(ReservationView.class)
            .timeout(Resilience.TIMEOUT);
    }

    public Mono<ReservationView> confirm(String reservationId) {
        return client.post().uri("/reservations/{id}/confirm", reservationId)
            .retrieve()
            .onStatus(HttpStatusCode::isError, response -> Mono.error(new DownstreamException("inventory", response.statusCode())))
            .bodyToMono(ReservationView.class)
            .timeout(Resilience.TIMEOUT)
            .retryWhen(Resilience.retry());
    }
}
