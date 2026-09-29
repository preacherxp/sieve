package shop.catalog;

import org.junit.jupiter.api.Test;
import reactor.test.StepVerifier;

class ProductRepositoryTest {
    private final ProductRepository repository = new ProductRepository();

    @Test
    void findsBySku() {
        StepVerifier.create(repository.findBySku("MUG-1").map(Product::name))
            .expectNext("Lambda mug")
            .verifyComplete();
    }

    @Test
    void listsCategorySortedBySku() {
        StepVerifier.create(repository.findByCategory("books").map(Product::sku))
            .expectNext("BOOK-1", "BOOK-2")
            .verifyComplete();
    }

    @Test
    void missingSkuIsEmpty() {
        StepVerifier.create(repository.findBySku("NOPE")).verifyComplete();
    }
}
