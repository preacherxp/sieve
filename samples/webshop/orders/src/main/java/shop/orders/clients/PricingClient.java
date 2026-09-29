package shop.orders.clients;

import java.util.Map;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.HttpStatusCode;
import org.springframework.stereotype.Component;
import org.springframework.web.reactive.function.client.WebClient;
import reactor.core.publisher.Mono;

@Component
public class PricingClient {
    private final WebClient client;

    public PricingClient(WebClient.Builder builder, @Value("${services.pricing}") String baseUrl) {
        this.client = builder.baseUrl(baseUrl).build();
    }

    public Mono<QuoteView> quote(ProductView product, int quantity) {
        var body = Map.of("sku", product.sku(), "category", product.category(), "unitPrice", product.price(), "quantity", quantity);
        return client.post().uri("/quotes").bodyValue(body)
            .retrieve()
            .onStatus(HttpStatusCode::isError, response -> Mono.error(new DownstreamException("pricing", response.statusCode())))
            .bodyToMono(QuoteView.class)
            .timeout(Resilience.TIMEOUT)
            .retryWhen(Resilience.retry());
    }
}
