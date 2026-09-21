package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class AsyncIT {
@Test void async() throws Exception {
 var executor = java.util.concurrent.Executors.newSingleThreadExecutor();
 try { assertEquals("done", executor.submit(() -> new AsyncWorker().work()).get(5, java.util.concurrent.TimeUnit.SECONDS)); }
 finally { executor.shutdownNow(); }
}
}
