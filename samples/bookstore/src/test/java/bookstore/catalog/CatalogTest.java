package bookstore.catalog;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import bookstore.catalog.Catalog;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class CatalogTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void searchesTitleAndAuthor() {
        Catalog catalog = new Catalog();
        catalog.add(TestEnvironment.DUNE);
        catalog.add(TestEnvironment.EMMA);
        assertEquals(1, catalog.search("austen").size());
        assertTrue(catalog.find("978-0441013593").isPresent());
    }
}
