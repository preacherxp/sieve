package bookstore.notify;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class EmailFormatterTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void formatsSubjectAndBody() {
        EmailFormatter formatter = new EmailFormatter();
        assertEquals("Your order: Dune", formatter.subject("Dune"));
        assertTrue(formatter.body("Ada", "Dune", 2).contains("2 x Dune"));
    }
}
