package shop.catalog;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import org.springframework.stereotype.Repository;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;
import shop.common.Money;

@Repository
public class ProductRepository {
    private final Map<String, Product> products = new ConcurrentHashMap<>();

    public ProductRepository() {
        save(new Product("BOOK-1", "Reactive Spring", "books", Money.of(3999)));
        save(new Product("BOOK-2", "Java Concurrency in Practice", "books", Money.of(4550)));
        save(new Product("MUG-1", "Lambda mug", "merch", Money.of(1250)));
        save(new Product("KBD-1", "Mechanical keyboard", "hardware", Money.of(12900)));
    }

    public Mono<Product> findBySku(String sku) {
        return Mono.justOrEmpty(products.get(sku));
    }

    public Flux<Product> findAll() {
        return Flux.fromIterable(products.values()).sort((a, b) -> a.sku().compareTo(b.sku()));
    }

    public Flux<Product> findByCategory(String category) {
        return findAll().filter(product -> product.category().equals(category));
    }

    public Mono<Product> save(Product product) {
        products.put(product.sku(), product);
        return Mono.just(product);
    }
}
