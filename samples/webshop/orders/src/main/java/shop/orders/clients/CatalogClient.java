package shop.orders.clients;

import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.HttpStatusCode;
import org.springframework.stereotype.Component;
import org.springframework.web.reactive.function.client.WebClient;
import reactor.core.publisher.Mono;

@Component
public class CatalogClient {
    private final WebClient client;

    public CatalogClient(WebClient.Builder builder, @Value("${services.catalog}") String baseUrl) {
        this.client = builder.baseUrl(baseUrl).build();
    }

    public Mono<ProductView> product(String sku) {
        return client.get().uri("/products/{sku}", sku)
            .retrieve()
            .onStatus(HttpStatusCode::isError, response -> Mono.error(new DownstreamException("catalog", response.statusCode())))
            .bodyToMono(ProductView.class)
            .timeout(Resilience.TIMEOUT)
            .retryWhen(Resilience.retry());
    }
}
