package bookstore.notify;

import static org.junit.jupiter.api.Assertions.*;

import bookstore.support.TestEnvironment;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class NotifierTest {
    @BeforeAll
    static void boot() throws InterruptedException {
        TestEnvironment.boot();
    }

    @Test
    void queuesOneEmailPerOrder() {
        Notifier notifier = new Notifier();
        notifier.orderPlaced("Ada", "Dune", 1);
        assertEquals(1, notifier.outbox().size());
        assertTrue(notifier.outbox().get(0).startsWith("Your order: Dune"));
    }
}
