package shop.catalog;

import org.springframework.http.HttpStatus;
import org.springframework.stereotype.Component;
import org.springframework.web.reactive.function.server.ServerRequest;
import org.springframework.web.reactive.function.server.ServerResponse;
import reactor.core.publisher.Mono;
import shop.common.web.ApiError;

@Component
public class CatalogHandler {
    private final ProductRepository repository;

    public CatalogHandler(ProductRepository repository) {
        this.repository = repository;
    }

    public Mono<ServerResponse> get(ServerRequest request) {
        String sku = request.pathVariable("sku");
        return repository.findBySku(sku)
            .flatMap(product -> ServerResponse.ok().bodyValue(product))
            .switchIfEmpty(Mono.defer(() -> ServerResponse.status(HttpStatus.NOT_FOUND)
                .bodyValue(new ApiError("PRODUCT_NOT_FOUND", "No product " + sku))));
    }

    public Mono<ServerResponse> list(ServerRequest request) {
        var products = request.queryParam("category")
            .map(repository::findByCategory)
            .orElseGet(repository::findAll);
        return ServerResponse.ok().body(products, Product.class);
    }
}
