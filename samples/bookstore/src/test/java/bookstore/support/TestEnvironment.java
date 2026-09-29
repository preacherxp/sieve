package bookstore.support;

import bookstore.catalog.Book;
import java.math.BigDecimal;

/**
 * Stands in for the expensive per-class setup of real suites, such as starting a database
 * container or an application context. Set -Dbookstore.setupMillis=0 to skip the wait.
 */
public final class TestEnvironment {
    public static final Book DUNE = new Book("978-0441013593", "Dune", "Frank Herbert", new BigDecimal("10.00"));
    public static final Book EMMA = new Book("978-0141439587", "Emma", "Jane Austen", new BigDecimal("8.00"));

    private TestEnvironment() {}

    public static void boot() throws InterruptedException {
        Thread.sleep(Long.getLong("bookstore.setupMillis", 1500));
    }
}
